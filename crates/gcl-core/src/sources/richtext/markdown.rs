//! The markdown side of [`super`]: a line scanner over a body whose tags are stripped first.

use super::html::{SKIPPED_IN_MARKDOWN, from_html};
use super::inline::{inline, inline_into};
use super::tokenize::{Token, attr, decode_entities, tokenize};
use super::{
    Block, MAX_BULLET_DEPTH, MAX_TABLE_COLS, MAX_TABLE_ROWS, capped, collapse, truncate_cell,
};

#[cfg(test)]
mod tests;

/// Converts markdown to blocks.
///
/// Headings come from leading `#` or a `===`/`---` underline, bullets from a leading
/// `-`, `*`, `+`, `1.`, or `1)` with their nesting read from the line's indent (two spaces
/// or one tab per level), code from a ``` ``` ``` fence, tables from a pipe row followed by
/// an alignment row, rules from a `---`, `***`, or `___` line of its own, and quotes from a
/// one-level `> ` line; every other non-empty run of lines is one paragraph, joined with
/// spaces. A paragraph that is one `**bold**` run and nothing else becomes a level-4
/// heading. `[text](url)` becomes `text (url)`, `![alt](url)` becomes a [`Block::Image`],
/// and `**`, `__`, and backtick markers are removed. A single `*x*` or `_x_` is stripped
/// too, but only when its markers sit at a word boundary, so `snake_case` keeps its
/// underscores: `check_this_out` has no letter-to-letter boundary for either underscore to
/// open or close on.
///
/// A real body carries HTML too — `<center>`, `<details>`, badge tables, `<br>`. Tags
/// outside a code fence are stripped first ([`strip_tags`]), which turns an `<img>` into
/// `![alt](src)`, a `<summary>` into a `###` heading line, and an `<hr>` into a `***` rule
/// line so the scanner sees them as markdown. A body that is block-level HTML with no
/// markdown structure left in it goes to [`from_html`] whole.
pub fn from_markdown(text: &str) -> Vec<Block> {
    if is_block_html(text) {
        return from_html(text);
    }
    capped(scan_markdown(&strip_tags_outside_fences(text)))
}

/// The paragraph being collected by [`scan_markdown`].
///
/// It keeps the raw lines beside the rewritten ones, because the "a whole paragraph in
/// bold is a subheading" rule reads markers [`inline`] has already removed, and it collects
/// the images those lines carried, which are blocks of their own emitted after it.
#[derive(Debug, Default)]
struct Para {
    lines: Vec<String>,
    raw: Vec<String>,
    images: Vec<Block>,
}

impl Para {
    /// Adds one line of prose.
    fn push(&mut self, line: &str) {
        self.raw.push(line.to_string());
        let text = inline_into(line, &mut self.images);
        self.lines.push(text);
    }

    /// Whether the paragraph is one `**bold**` run and nothing else.
    fn is_subheading(&self) -> bool {
        self.raw.len() == 1 && is_bold_only(self.raw[0].trim())
    }

    /// Takes the collected text, leaving the images for [`Para::drain_images`].
    fn take_text(&mut self) -> String {
        let text = self.lines.join(" ");
        self.lines.clear();
        collapse(&text)
    }

    /// Moves the images the paragraph carried into `blocks`.
    fn drain_images(&mut self, blocks: &mut Vec<Block>) {
        self.raw.clear();
        blocks.append(&mut self.images);
    }

    /// Ends the paragraph, dropping it when it holds no text.
    fn flush(&mut self, blocks: &mut Vec<Block>) {
        let subheading = self.is_subheading();
        let text = self.take_text();
        if !text.is_empty() {
            blocks.push(match subheading {
                true => Block::Heading(4, text),
                false => Block::Paragraph(text),
            });
        }
        self.drain_images(blocks);
    }
}

/// The line scanner behind [`from_markdown`], over text whose tags are already stripped.
fn scan_markdown(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut para = Para::default();
    let mut quote = Quote::default();
    let mut code: Option<Vec<String>> = None;
    let mut list_indents: Vec<usize> = Vec::new();
    let mut at = 0;

    while at < lines.len() {
        let raw = lines[at].trim_end();
        at += 1;
        if let Some(collected) = code.as_mut() {
            if raw.trim_start().starts_with("```") {
                push_code(&mut blocks, std::mem::take(collected));
                code = None;
            } else {
                collected.push(raw.to_string());
            }
            continue;
        }
        if let Some((table, next)) = table_at(&lines, at - 1) {
            para.flush(&mut blocks);
            quote.flush(&mut blocks);
            list_indents.clear();
            blocks.push(table);
            at = next;
            continue;
        }
        let head = raw.trim_start();
        if let Some(body) = quote_body(head) {
            para.flush(&mut blocks);
            list_indents.clear();
            quote.push(body);
            continue;
        }
        quote.flush(&mut blocks);
        let trimmed = unquote(head);
        if trimmed.starts_with("```") {
            para.flush(&mut blocks);
            list_indents.clear();
            code = Some(Vec::new());
        } else if trimmed.is_empty() {
            para.flush(&mut blocks);
        } else if let Some(kind) = line_rule(trimmed) {
            // `===` or `---` under a text line is a setext heading; on its own either is a
            // rule, as are `***` and `___`. A row of `|`, `-`, and `:` that no table row
            // precedes is a stray separator and is dropped.
            list_indents.clear();
            let subheading = para.is_subheading();
            let text = para.take_text();
            match (kind, text.is_empty()) {
                (LineRule::Setext(level), false) => blocks.push(Block::Heading(level, text)),
                (LineRule::Setext(_), true) => blocks.push(Block::Rule),
                (kind, empty) => {
                    if !empty {
                        blocks.push(match subheading {
                            true => Block::Heading(4, text),
                            false => Block::Paragraph(text),
                        });
                    }
                    if kind == LineRule::Rule {
                        blocks.push(Block::Rule);
                    }
                }
            }
            para.drain_images(&mut blocks);
        } else if let Some(rest) = trimmed.strip_prefix('#') {
            para.flush(&mut blocks);
            list_indents.clear();
            let extra = rest.chars().take_while(|c| *c == '#').count();
            let level = (1 + extra).min(6) as u8;
            let body = inline(rest.trim_start_matches('#').trim());
            if !body.is_empty() {
                blocks.push(Block::Heading(level, body));
            }
        } else if let Some(rest) = bullet_body(trimmed) {
            para.flush(&mut blocks);
            let mut images = Vec::new();
            let body = inline_into(rest.trim(), &mut images);
            if !body.is_empty() {
                blocks.push(Block::Bullet {
                    depth: bullet_depth(&mut list_indents, indent_of(raw)),
                    text: body,
                });
            }
            blocks.append(&mut images);
        } else {
            para.push(trimmed);
        }
    }
    if let Some(collected) = code {
        push_code(&mut blocks, collected);
    }
    quote.flush(&mut blocks);
    para.flush(&mut blocks);
    blocks
}

/// How far a line is indented, in columns: one per space and two per tab.
///
/// A tab counts as the two columns a markdown list nests by, so a tab-indented item sits
/// one level in rather than two.
fn indent_of(line: &str) -> usize {
    let mut columns = 0usize;
    for c in line.chars() {
        match c {
            ' ' => columns += 1,
            '\t' => columns += 2,
            _ => break,
        }
    }
    columns
}

/// How deeply a list item is nested, from the indent columns seen so far in this list.
///
/// `seen` is the stack of columns the open levels start at. A deeper indent than the top
/// pushes one level, whatever its width, so a list written with four spaces per step nests
/// one level per step and not two; a shallower one pops back to the level it matches. The
/// depth is capped at [`MAX_BULLET_DEPTH`].
fn bullet_depth(seen: &mut Vec<usize>, indent: usize) -> u8 {
    while seen.last().is_some_and(|top| indent < *top) {
        seen.pop();
    }
    if seen.last().is_none_or(|top| indent > *top) {
        seen.push(indent);
    }
    (seen.len().saturating_sub(1)).min(MAX_BULLET_DEPTH as usize) as u8
}

/// The text of a clean one-level `> ` quote line, or `None`.
///
/// A nested `>>` is not one: it keeps the old behaviour, where [`unquote`] strips the
/// markers and the text joins the surrounding paragraph.
fn quote_body(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('>')?.trim_start();
    match rest.starts_with('>') {
        true => None,
        false => Some(rest),
    }
}

/// The block quote being collected by [`scan_markdown`].
///
/// It keeps the images its lines carried beside the text, the way [`Para`] does: an image
/// is a block of its own, emitted after the quote it sat in, rather than lost with the
/// markup around it.
#[derive(Debug, Default)]
struct Quote {
    lines: Vec<String>,
    images: Vec<Block>,
}

impl Quote {
    /// Adds one quoted line.
    fn push(&mut self, body: &str) {
        let text = inline_into(body, &mut self.images);
        self.lines.push(text);
    }

    /// Ends the quote, pushing it and its images, if any line carried text.
    fn flush(&mut self, blocks: &mut Vec<Block>) {
        let text = collapse(&std::mem::take(&mut self.lines).join(" "));
        if !text.is_empty() {
            blocks.push(Block::Quote(text));
        }
        blocks.append(&mut self.images);
    }
}

/// Whether a paragraph is one `**bold**` or `__bold__` run and nothing else.
fn is_bold_only(text: &str) -> bool {
    for marker in ["**", "__"] {
        if let Some(rest) = text.strip_prefix(marker)
            && let Some(inner) = rest.strip_suffix(marker)
            && !inner.trim().is_empty()
            && !inner.contains(marker)
        {
            return true;
        }
    }
    false
}

/// The table starting at line `at`, and the index just past its last row, or `None`.
///
/// A table is a pipe row whose next line is an alignment row (`|---|---|`): that row is
/// what marks the line above it as the header. Body rows run until the first line that is
/// no longer a pipe row. Extra columns past [`MAX_TABLE_COLS`] and rows past
/// [`MAX_TABLE_ROWS`] are dropped.
fn table_at(lines: &[&str], at: usize) -> Option<(Block, usize)> {
    let head = lines.get(at)?.trim();
    if !is_table_row(head) {
        return None;
    }
    let separator = lines.get(at + 1)?.trim();
    if line_rule(separator) != Some(LineRule::TableSeparator) {
        return None;
    }
    let header = table_cells(head);
    if header.is_empty() {
        return None;
    }
    let mut rows = Vec::new();
    let mut next = at + 2;
    while let Some(line) = lines.get(next) {
        let line = line.trim();
        if !is_table_row(line) {
            break;
        }
        if rows.len() < MAX_TABLE_ROWS {
            let mut cells = table_cells(line);
            cells.resize(header.len(), String::new());
            rows.push(cells);
        }
        next += 1;
    }
    Some((Block::Table { header, rows }, next))
}

/// Whether a line is a pipe-table row: it carries a `|` that is not escaped.
///
/// Neither outer pipes nor spaces around them are needed, so `a|b` is a row. What keeps a
/// sentence with a pipe in it from becoming a table is [`table_at`], which asks for an
/// alignment row underneath before it reads either line as one.
fn is_table_row(line: &str) -> bool {
    split_cells(line).len() > 1
}

/// Splits a row on its unescaped `|`, keeping a `\\|` inside the cell it sits in.
fn split_cells(line: &str) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut escaped = false;
    for c in line.chars() {
        match c {
            '\\' if !escaped => escaped = true,
            '|' if !escaped => cells.push(String::new()),
            c => {
                if escaped
                    && c != '|'
                    && let Some(cell) = cells.last_mut()
                {
                    cell.push('\\');
                }
                escaped = false;
                if let Some(cell) = cells.last_mut() {
                    cell.push(c);
                }
            }
        }
    }
    cells
}

/// Splits one pipe-table row into its cells, inline markup rewritten and each cell cut at
/// [`MAX_CELL_CHARS`].
fn table_cells(line: &str) -> Vec<String> {
    let mut cells = split_cells(line);
    // Outer pipes are optional, and when they are there they open and close an empty cell.
    if line.starts_with('|') && !cells.is_empty() {
        cells.remove(0);
    }
    if line.ends_with('|') && !line.ends_with("\\|") {
        cells.pop();
    }
    cells
        .into_iter()
        .take(MAX_TABLE_COLS)
        .map(|cell| {
            let mut text = inline(cell.trim());
            truncate_cell(&mut text);
            text
        })
        .collect()
}

/// The text after a list marker, or `None` when the line is not a list item.
///
/// A `-`, `*`, or `+` marker and an ordered `1.` / `1)` marker both count: an ordered list
/// is displayed as bullets, since a [`Block`] carries no numbering.
fn bullet_body(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest);
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &line[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(rest);
        }
    }
    match line {
        "-" | "*" | "+" => Some(""),
        _ => None,
    }
}

/// Drops every leading `>` blockquote marker from a line.
fn unquote(line: &str) -> &str {
    let mut rest = line;
    while let Some(stripped) = rest.strip_prefix('>') {
        rest = stripped.trim_start();
    }
    rest
}

/// A line that is markup on its own rather than prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineRule {
    /// A `===` or `---` underline, which makes the line above it a heading of this level.
    Setext(u8),
    /// A `***` or `___` line, which is always a rule.
    Rule,
    /// A table alignment row: only `|`, `-`, and `:` in it.
    TableSeparator,
}

/// Which markup a whole line is, or `None` when it is prose.
fn line_rule(line: &str) -> Option<LineRule> {
    let body: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if body.len() < 2 {
        return None;
    }
    if body.chars().all(|c| c == '=') {
        return Some(LineRule::Setext(1));
    }
    if body.chars().all(|c| c == '-') {
        return Some(LineRule::Setext(2));
    }
    if body.len() >= 3 && (body.chars().all(|c| c == '*') || body.chars().all(|c| c == '_')) {
        return Some(LineRule::Rule);
    }
    if body.contains('|') && body.chars().all(|c| matches!(c, '|' | '-' | ':')) {
        return Some(LineRule::TableSeparator);
    }
    None
}

/// Pushes the collected fence lines as one code block, if any carry text.
fn push_code(blocks: &mut Vec<Block>, lines: Vec<String>) {
    let text = lines.join("\n").trim_matches('\n').to_string();
    if !text.trim().is_empty() {
        blocks.push(Block::Code(text));
    }
}

/// Tags that start a new line when a markdown body's tags are stripped.
const BLOCK_TAGS: [&str; 16] = [
    "p",
    "div",
    "ul",
    "ol",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "center",
    "details",
    "summary",
    "section",
    "figure",
];

/// Whether a body is block-level HTML rather than markdown.
///
/// True when it carries a block-level tag and no markdown structure at all — no `#`
/// heading, list marker, or code fence on any line. Such a body is handed to
/// [`from_html`] whole, which keeps its headings and lists; a mixed body (markdown
/// headings with `<p>` and badge images in it, the common Modrinth shape) stays on the
/// markdown path with its tags stripped.
fn is_block_html(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_block = [
        "<p>", "<p ", "<ul", "<ol", "<li", "<h1", "<h2", "<h3", "<h4", "<h5", "<h6",
    ]
    .iter()
    .any(|tag| lower.contains(tag));
    if !has_block {
        return false;
    }
    !text.lines().any(|line| {
        let line = unquote(line.trim_start());
        line.starts_with('#') || line.starts_with("```") || bullet_body(line).is_some()
    })
}

/// Strips HTML tags from every part of `text` that is not inside a code fence.
fn strip_tags_outside_fences(text: &str) -> String {
    let mut out = String::new();
    let mut plain = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if !in_fence {
                out.push_str(&strip_tags(&std::mem::take(&mut plain)));
            }
            out.push_str(line);
            out.push('\n');
            in_fence = !in_fence;
            continue;
        }
        let target = if in_fence { &mut out } else { &mut plain };
        target.push_str(line);
        target.push('\n');
    }
    out.push_str(&strip_tags(&plain));
    out
}

/// Drops every tag from `html` and keeps its text, entities decoded, rewritten as the
/// markdown [`scan_markdown`] reads next.
///
/// The same tokenizer [`from_html`] uses: the subtree of a [`SKIPPED_IN_MARKDOWN`] tag is
/// dropped, a block-level tag becomes a line break, an `<img>` becomes `![alt](src)`, a
/// `<summary>` becomes a `###` heading line, an `<hr>` becomes a `***` rule line, a `<li>`
/// becomes a `-` bullet marker indented by its list nesting, and two `<br>` in a row become
/// a blank line so the paragraph ends there.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut skip: Option<(String, u32)> = None;
    let mut breaks = 0u32;
    let mut list_depth = 0usize;
    for token in tokenize(html) {
        if let Some((tag, depth)) = skip.as_mut() {
            if let Token::Tag { name, close, .. } = &token
                && name == tag
            {
                if *close {
                    *depth -= 1;
                    if *depth == 0 {
                        skip = None;
                    }
                } else {
                    *depth += 1;
                }
            }
            continue;
        }
        match token {
            Token::Text(text) => {
                if text.chars().any(|c| !c.is_whitespace()) {
                    breaks = 0;
                }
                out.push_str(&decode_entities(&text));
            }
            Token::Tag {
                name,
                close,
                self_closing,
                attrs,
            } => {
                if SKIPPED_IN_MARKDOWN.contains(&name.as_str()) {
                    if !close && !self_closing {
                        skip = Some((name, 1));
                    }
                    out.push('\n');
                    continue;
                }
                if name == "br" {
                    breaks += 1;
                    match breaks {
                        1 => out.push(' '),
                        2 => out.push_str("\n\n"),
                        _ => {}
                    }
                    continue;
                }
                breaks = 0;
                match name.as_str() {
                    "img" => out.push_str(&image_markdown(&attrs)),
                    "summary" if !close => out.push_str("\n### "),
                    "hr" => out.push_str("\n\n***\n\n"),
                    "ul" | "ol" => {
                        list_depth = match close {
                            true => list_depth.saturating_sub(1),
                            false => list_depth + 1,
                        };
                        out.push('\n');
                    }
                    "li" if !close => {
                        out.push('\n');
                        for _ in 1..list_depth.max(1) {
                            out.push_str("  ");
                        }
                        out.push_str("- ");
                    }
                    "li" | "tr" => out.push('\n'),
                    name if BLOCK_TAGS.contains(&name) => out.push('\n'),
                    _ => {}
                }
            }
        }
    }
    out
}

/// An `<img>`'s attributes as a markdown image on a line of its own, or an empty string
/// when it carries no `src`.
///
/// The alt text loses the characters that would end the label early, so a caption with a
/// bracket in it cannot break the link the scanner reads back.
fn image_markdown(attrs: &str) -> String {
    let src = decode_entities(&attr(attrs, "src"));
    let src = src.trim();
    if src.is_empty() || src.contains(char::is_whitespace) {
        return String::new();
    }
    // A `(` or `)` in the URL would end the markdown link early once the scanner reads
    // this line back, so both are percent-encoded, which names the same file.
    let src = src.replace('(', "%28").replace(')', "%29");
    let alt = collapse(&decode_entities(&attr(attrs, "alt"))).replace(['[', ']'], " ");
    format!("\n![{}]({src})\n", collapse(&alt))
}
