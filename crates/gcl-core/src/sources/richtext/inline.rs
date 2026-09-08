//! Inline markdown markup: links, images, emphasis, and inline code.

use super::tokenize::starts_with;
use super::{Block, collapse};

#[cfg(test)]
mod tests;

/// [`inline_into`] for a caller with nowhere to put images, such as a heading or a table
/// cell: any image the text carried is dropped.
pub(super) fn inline(text: &str) -> String {
    inline_into(text, &mut Vec::new())
}

/// Rewrites markdown inline markup for display: links flattened to `text (url)`, emphasis
/// and inline-code markers removed, and every `![alt](url)` moved into `images` as a
/// [`Block::Image`], since an image is a block of its own rather than part of the line.
///
/// A badge — an image that is a link's whole label — is dropped outright, image and all.
pub(super) fn inline_into(text: &str, images: &mut Vec<Block>) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    // A line of unmatched `[` would otherwise rescan the rest of the line for every one of
    // them. Past the last `]` no link can start, so the scan is skipped outright.
    let last_close = chars.iter().rposition(|c| *c == ']');
    let may_link = |at: usize| last_close.is_some_and(|close| at < close);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '!'
            && may_link(i + 1)
            && let Some(end) = link_at(&chars, i + 1)
        {
            let (alt, url) = split_link(&chars, i + 1, end);
            if !url.is_empty() {
                images.push(Block::Image {
                    url,
                    alt: inline(&alt),
                });
            }
            i = end;
            continue;
        }
        // `[![alt](image)](url)`: a badge, which is an image and nothing else. Both the
        // image and the link around it go, the same way an anchor with no text keeps no URL.
        if chars[i] == '['
            && chars.get(i + 1) == Some(&'!')
            && may_link(i + 2)
            && let Some(end) = badge_at(&chars, i)
        {
            i = end;
            continue;
        }
        if chars[i] == '['
            && may_link(i)
            && let Some(end) = link_at(&chars, i)
        {
            let (label, url) = split_link(&chars, i, end);
            out.push_str(&inline(&label));
            if !url.is_empty() {
                out.push_str(&format!(" ({url})"));
            }
            i = end;
            continue;
        }
        if starts_with(&chars, i, "**") || starts_with(&chars, i, "__") {
            i += 2;
            continue;
        }
        if (chars[i] == '_' || chars[i] == '*')
            && let Some(end) = single_emphasis_at(&chars, i, chars[i])
        {
            let label: String = chars[i + 1..end - 1].iter().collect();
            out.push_str(&inline(&label));
            i = end;
            continue;
        }
        if chars[i] == '`' {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    collapse(&out)
}

/// A letter, digit, or underscore: what keeps `_` inside `snake_case` from reading as an
/// emphasis marker.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The index just past a single-character emphasis span (`_x_` or `*x*`) opening at `at`,
/// or `None` when `at` does not open one.
///
/// Both ends have to sit at a word boundary: the character right after the opening marker,
/// and the one right before the closing marker, must not be whitespace, and the character
/// right before the opening marker and right after the closing marker (if either exists)
/// must not be a [word character](is_word_char). That last rule is what keeps
/// `check_updates_button` intact — the underscore between `check` and `updates` has a
/// letter on both sides, so it never opens a span — while still stripping `_important_`.
/// The closing marker is looked for within [`LINK_SCAN`] characters, the same bound
/// [`link_at`] uses.
fn single_emphasis_at(chars: &[char], at: usize, marker: char) -> Option<usize> {
    if chars.get(at) != Some(&marker) {
        return None;
    }
    if at > 0 && is_word_char(chars[at - 1]) {
        return None;
    }
    match chars.get(at + 1) {
        Some(c) if !c.is_whitespace() && *c != marker => {}
        _ => return None,
    }
    let end = (at + 1..chars.len().min(at + 1 + LINK_SCAN)).find(|&j| {
        chars[j] == marker
            && !chars[j - 1].is_whitespace()
            && chars.get(j + 1).is_none_or(|c| !is_word_char(*c))
    })?;
    Some(end + 1)
}

/// How far past a `[` the label and the URL are looked for.
const LINK_SCAN: usize = 512;

/// The index just past a `[label](url)` starting at `at`, or `None`.
///
/// The label and the URL are each looked for within [`LINK_SCAN`] characters, so an
/// unmatched `[` costs a bounded scan rather than the rest of the line.
fn link_at(chars: &[char], at: usize) -> Option<usize> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let close = (at + 1..chars.len().min(at + 1 + LINK_SCAN)).find(|i| chars[*i] == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len().min(close + 2 + LINK_SCAN)).find(|i| chars[*i] == ')')?;
    Some(end + 1)
}

/// The index just past a `[![alt](image)](url)` badge starting at `at`, or `None`.
fn badge_at(chars: &[char], at: usize) -> Option<usize> {
    let image_end = link_at(chars, at + 2)?;
    if chars.get(image_end) != Some(&']') || chars.get(image_end + 1) != Some(&'(') {
        return None;
    }
    let end =
        (image_end + 2..chars.len().min(image_end + 2 + LINK_SCAN)).find(|i| chars[*i] == ')')?;
    Some(end + 1)
}

/// Splits a `[label](url)` span into its label and its URL.
fn split_link(chars: &[char], start: usize, end: usize) -> (String, String) {
    let span: String = chars[start..end].iter().collect();
    let label = span
        .find(']')
        .map(|i| span[1..i].to_string())
        .unwrap_or_default();
    let url = span
        .find("](")
        .map(|i| span[i + 2..span.len() - 1].to_string())
        .unwrap_or_default();
    // A markdown link may carry a title after the URL: `(url "title")`. Only the URL
    // is displayed.
    let url = url.split_whitespace().next().unwrap_or("").to_string();
    (label, url)
}
