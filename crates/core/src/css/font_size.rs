//! Keep the reader's text size setting in charge of body text.
//!
//! CrossPoint 2.0 honors `font-size` (`CssParser.cpp:884-930` at firmware
//! 2.0.0). `rem`, `px`, `pt` and the absolute keywords scale the reader's own
//! size; `em`, `%`, `ex`/`ch` and `smaller`/`larger` scale the parent
//! (`ChapterHtmlSlimParser.cpp:3015-3031`). Many books set a size on `html` or
//! `body` and a size near one on every paragraph, which would pin the body
//! text instead of letting the reader choose. So the subset filter:
//!
//! - drops `font-size` on a rule whose subject is `html` or `body` (and on
//!   their inline styles);
//! - rescales `rem` by the book's root size: with `html{font-size:62.5%}`,
//!   `1.6rem` means the browser's 16px and becomes `1rem` (the device reads
//!   `rem` as the reader's size, not the book's root);
//! - drops a size that resolves to 0.85..=1.15 of its reference, so ordinary
//!   text keeps the reader's size, while headings, small print and drop caps
//!   keep theirs. Not on `h1`-`h3`, `h5`, `h6`, `small` and `big`: the device
//!   gives those a default size of their own when none is set
//!   (`ChapterHtmlSlimParser.cpp:2267, 3024-3027`), so a `1em` there is a real
//!   change.

use lightningcss::properties::Property;
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;

use super::subset::media_is_screen_or_all;

/// A size within this band of its reference is ordinary text and dropped.
const NEAR_ONE: std::ops::RangeInclusive<f64> = 0.85..=1.15;

/// The device's font-size keywords and their scale (`CssParser.cpp:892-897`).
const KEYWORDS: &[(&str, f64)] = &[
    ("xx-small", 0.6),
    ("x-small", 0.75),
    ("small", 0.89),
    ("medium", 1.0),
    ("large", 1.2),
    ("x-large", 1.5),
    ("xx-large", 2.0),
    ("xxx-large", 3.0),
    ("smaller", 0.83),
    ("larger", 1.2),
];

/// Tags the device gives a default size of their own when no `font-size` is
/// set (h4's default is 1.0, so it is not one of them).
const DEFAULT_SIZED_TAGS: &[&str] = &["h1", "h2", "h3", "h5", "h6", "small", "big"];

/// What a `font-size` applies to, by the subject element's tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SizeSubject {
    /// `html` or `body`: the size is dropped.
    Root,
    /// A tag with a default size of its own: the size is kept as written.
    DefaultSized,
    /// Anything else: a size near one is dropped.
    Plain,
}

impl SizeSubject {
    /// The subject kind of an element named `tag` (any ASCII case).
    pub(crate) fn of_tag(tag: &str) -> Self {
        let tag = tag.to_ascii_lowercase();
        if tag == "html" || tag == "body" {
            Self::Root
        } else if DEFAULT_SIZED_TAGS.contains(&tag.as_str()) {
            Self::DefaultSized
        } else {
            Self::Plain
        }
    }

    /// The subject kind of a device-legal serialized selector: the tag of its
    /// last compound. A class-only subject counts as plain.
    pub(crate) fn of_selector(selector: &str) -> Self {
        let chain = super::subset::strip_first_letter(selector);
        let subject = chain.rsplit(' ').next().unwrap_or(chain);
        let end = subject.find(['#', '.', ':']).unwrap_or(subject.len());
        Self::of_tag(&subject[..end])
    }
}

/// The value `font-size: value` should reach the device as on `subject`, or
/// `None` to drop it. `rem_scale` is the book's root size as a factor of
/// 16px (1.0 for none); `rem` values are multiplied by it.
pub(crate) fn device_font_size(
    value: &str,
    subject: SizeSubject,
    rem_scale: f64,
) -> Option<String> {
    if subject == SizeSubject::Root {
        return None;
    }
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    // The device cannot parse these, so they are dead weight anyway.
    if matches!(
        lower.as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    ) {
        return None;
    }
    let (scale, rewritten) = if let Some(&(_, scale)) =
        KEYWORDS.iter().find(|(name, _)| *name == lower)
    {
        (scale, None)
    } else {
        let unit_start = lower
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-')))
            .unwrap_or(lower.len());
        let Ok(number) = lower[..unit_start].parse::<f64>() else {
            return Some(value.to_string());
        };
        match &lower[unit_start..] {
            "" | "px" => (number / 16.0, None),
            "pt" => (number / 12.0, None),
            "em" => (number, None),
            "%" => (number / 100.0, None),
            "ex" | "ch" => (number / 2.0, None),
            "rem" => {
                let scaled = number * rem_scale;
                let rewritten = (rem_scale != 1.0).then(|| format!("{}rem", format_number(scaled)));
                (scaled, rewritten)
            }
            _ => return Some(value.to_string()),
        }
    };
    if subject == SizeSubject::Plain && NEAR_ONE.contains(&round(scale)) {
        return None;
    }
    Some(rewritten.unwrap_or_else(|| value.to_string()))
}

/// The book's root font size as a factor of the browser's 16px, from the last
/// `html` or `:root` rule in `css` that sets a `%`, `px`, `pt`, `em` or `rem`
/// size. `None` when the sheet sets none (or cannot be parsed).
pub(crate) fn root_font_scale(css: &str) -> Option<f64> {
    let options = ParserOptions {
        error_recovery: true,
        ..Default::default()
    };
    let sheet = StyleSheet::parse(css, options).ok()?;
    root_font_scale_of(&sheet.rules.0)
}

/// [`root_font_scale`] over already parsed rules, looking into the `@media`
/// blocks the filter keeps (`screen` and `all`) but not print or conditional ones.
pub(crate) fn root_font_scale_of(rules: &[CssRule<'_>]) -> Option<f64> {
    let mut found = None;
    for rule in rules {
        match rule {
            CssRule::Style(style) => {
                let is_root = style.selectors.0.iter().any(|selector| {
                    selector
                        .to_css_string(PrinterOptions::default())
                        .is_ok_and(|s| s.eq_ignore_ascii_case("html") || s == ":root")
                });
                if !is_root {
                    continue;
                }
                for property in style
                    .declarations
                    .declarations
                    .iter()
                    .chain(style.declarations.important_declarations.iter())
                {
                    if let Property::FontSize(_) = property
                        && let Ok(value) = property.value_to_css_string(PrinterOptions::default())
                        && let Some(scale) = absolute_scale(&value)
                    {
                        found = Some(scale);
                    }
                }
            }
            CssRule::Media(media) if media_is_screen_or_all(&media.query) => {
                if let Some(scale) = root_font_scale_of(&media.rules.0) {
                    found = Some(scale);
                }
            }
            _ => {}
        }
    }
    found
}

/// A root `font-size` as a factor of 16px, for the units a root size is
/// written in. A non-positive or unreadable size gives `None`.
fn absolute_scale(value: &str) -> Option<f64> {
    let lower = value.trim().to_ascii_lowercase();
    let unit_start = lower
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-')))
        .unwrap_or(lower.len());
    let number: f64 = lower[..unit_start].parse().ok()?;
    let scale = match &lower[unit_start..] {
        "%" => number / 100.0,
        "px" => number / 16.0,
        "pt" => number / 12.0,
        "em" | "rem" => number,
        _ => return None,
    };
    (scale > 0.0).then_some(scale)
}

/// `x` rounded to 4 decimals, so float noise (`1.6 * 0.625`) does not move a
/// value across a band edge.
fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// `x` with at most 4 decimals and no trailing zeros: `1.0` -> `1`,
/// `0.875` -> `0.875`.
fn format_number(x: f64) -> String {
    let text = format!("{:.4}", round(x));
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(value: &str, scale: f64) -> Option<String> {
        device_font_size(value, SizeSubject::Plain, scale)
    }

    #[test]
    fn root_subjects_lose_every_size() {
        for value in ["2em", "62.5%", "16px", "large"] {
            assert_eq!(device_font_size(value, SizeSubject::Root, 1.0), None);
        }
    }

    #[test]
    fn near_one_sizes_are_dropped() {
        for value in [
            "0.9em", "1em", "1.15em", "100%", "85%", "16px", "12pt", "1rem", "medium", "small",
            "inherit", "initial", "unset",
        ] {
            assert_eq!(plain(value, 1.0), None, "{value}");
        }
    }

    #[test]
    fn other_sizes_are_kept() {
        for value in [
            "2em", "0.75em", "1.2em", "84%", "116%", "12px", "large", "smaller", "x-small",
        ] {
            assert_eq!(plain(value, 1.0).as_deref(), Some(value), "{value}");
        }
    }

    #[test]
    fn rem_is_rescaled_by_the_root_size() {
        assert_eq!(plain("1.6rem", 0.625), None, "1.6rem at 62.5% is 1rem");
        assert_eq!(plain("3.2rem", 0.625).as_deref(), Some("2rem"));
        assert_eq!(plain("1.2rem", 0.625).as_deref(), Some("0.75rem"));
        assert_eq!(plain("2rem", 1.0).as_deref(), Some("2rem"));
        // Absolute units are not relative to the root.
        assert_eq!(plain("12px", 0.625).as_deref(), Some("12px"));
    }

    #[test]
    fn default_sized_tags_keep_a_near_one_size() {
        for tag in ["h1", "H2", "h3", "h5", "h6", "small", "big"] {
            let subject = SizeSubject::of_tag(tag);
            assert_eq!(subject, SizeSubject::DefaultSized, "{tag}");
            assert_eq!(
                device_font_size("1em", subject, 1.0).as_deref(),
                Some("1em")
            );
        }
        assert_eq!(SizeSubject::of_tag("h4"), SizeSubject::Plain);
    }

    #[test]
    fn selector_subject_is_the_last_compound() {
        assert_eq!(SizeSubject::of_selector("body"), SizeSubject::Root);
        assert_eq!(SizeSubject::of_selector("BODY.x"), SizeSubject::Root);
        assert_eq!(SizeSubject::of_selector("html body"), SizeSubject::Root);
        assert_eq!(SizeSubject::of_selector("body p"), SizeSubject::Plain);
        assert_eq!(
            SizeSubject::of_selector("div > h1#t"),
            SizeSubject::DefaultSized
        );
        assert_eq!(
            SizeSubject::of_selector("p:first-letter"),
            SizeSubject::Plain
        );
        assert_eq!(SizeSubject::of_selector(".title"), SizeSubject::Plain);
    }

    #[test]
    fn root_scale_comes_from_the_last_html_or_root_rule() {
        assert_eq!(root_font_scale("html{font-size:62.5%}"), Some(0.625));
        assert_eq!(root_font_scale(":root{font-size:10px}"), Some(0.625));
        assert_eq!(
            root_font_scale("html{font-size:50%}@media screen{html{font-size:0.75em}}"),
            Some(0.75)
        );
        assert_eq!(root_font_scale("html, p{font-size:12pt}"), Some(1.0));
        assert_eq!(root_font_scale("body{font-size:62.5%}"), None);
        assert_eq!(root_font_scale("html{font-size:large}"), None);
        assert_eq!(root_font_scale("html{margin:0}"), None);
    }

    #[test]
    fn root_scale_ignores_media_the_filter_drops() {
        // The device never sees a print or conditional block, so it cannot
        // set the root size (`filter_css` keeps only `screen` and `all`).
        assert_eq!(root_font_scale("@media print{html{font-size:62.5%}}"), None);
        assert_eq!(
            root_font_scale("@media (min-width:600px){html{font-size:62.5%}}"),
            None
        );
        assert_eq!(
            root_font_scale("html{font-size:75%}@media print{html{font-size:62.5%}}"),
            Some(0.75)
        );
        assert_eq!(
            root_font_scale("@media all{html{font-size:62.5%}}"),
            Some(0.625)
        );
    }
}
