//! Project descriptions as a short list of display blocks.
//!
//! Modrinth serves a description as markdown (`body` on the project) and CurseForge
//! serves it as HTML (`GET /v1/mods/{id}/description`). Both become the same
//! [`Block`] list so the UI renders one shape. This is a display converter, not a
//! markdown or HTML implementation: it keeps headings, paragraphs, bullets, code
//! blocks, tables, rules, quotes, images, and link URLs, and strips everything else
//! down to its text.
//!
//! No new dependency: the markdown side is a line scanner, the HTML side a small
//! tag-aware stripper.

#[cfg(test)]
mod tests;

/// One piece of a rendered description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A heading and its level, 1 to 6.
    Heading(u8, String),
    /// A run of prose.
    Paragraph(String),
    /// One list item and how deeply it is nested, 0 for a top-level item.
    Bullet { depth: u8, text: String },
    /// A code block, newlines kept.
    Code(String),
    /// A table: its header cells and its body rows, every row as wide as the header.
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// A horizontal rule.
    Rule,
    /// An image and its alt text, which may be empty.
    Image { url: String, alt: String },
    /// A block quote, one level deep.
    Quote(String),
}

impl Block {
    /// The block's text, without its kind.
    ///
    /// A [`Block::Rule`] and a [`Block::Table`] have no single text of their own and
    /// answer `""` — a table's strings are its `header` and `rows`. A [`Block::Image`]
    /// answers its alt text, which is what a renderer shows when the image is missing.
    pub fn text(&self) -> &str {
        match self {
            Block::Heading(_, text)
            | Block::Paragraph(text)
            | Block::Bullet { text, .. }
            | Block::Code(text)
            | Block::Quote(text)
            | Block::Image { alt: text, .. } => text,
            Block::Rule | Block::Table { .. } => "",
        }
    }

    /// The block's text for editing, or `None` when it carries none of its own.
    fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Block::Heading(_, text)
            | Block::Paragraph(text)
            | Block::Bullet { text, .. }
            | Block::Code(text)
            | Block::Quote(text)
            | Block::Image { alt: text, .. } => Some(text),
            Block::Rule | Block::Table { .. } => None,
        }
    }
}

/// Most blocks one description may produce.
const MAX_BLOCKS: usize = 2_000;

/// Most bytes one block's text may hold.
const MAX_BLOCK_BYTES: usize = 8 * 1024;

/// Most body rows one table may hold.
const MAX_TABLE_ROWS: usize = 50;

/// Most columns one table may hold.
const MAX_TABLE_COLS: usize = 8;

/// Most characters one table cell may hold.
const MAX_CELL_CHARS: usize = 200;

/// Deepest bullet nesting a list item may report.
const MAX_BULLET_DEPTH: u8 = 6;

/// Marks text a cap cut short.
const ELLIPSIS: char = '…';

/// Applies the output caps: [`MAX_BLOCKS`] blocks, [`MAX_BLOCK_BYTES`] per block.
///
/// A description is display data, so a body that runs long is cut rather than refused: an
/// over-long block ends in an ellipsis, and a body over the block cap ends with one
/// `Paragraph("…")`.
fn capped(mut blocks: Vec<Block>) -> Vec<Block> {
    let over = blocks.len() > MAX_BLOCKS;
    blocks.truncate(MAX_BLOCKS);
    for block in &mut blocks {
        if let Some(text) = block.text_mut() {
            truncate_text(text);
        }
    }
    if over {
        blocks.push(Block::Paragraph(ELLIPSIS.to_string()));
    }
    blocks
}

/// Cuts `text` to [`MAX_BLOCK_BYTES`] on a character boundary, marking the cut.
fn truncate_text(text: &mut String) {
    if text.len() > MAX_BLOCK_BYTES {
        cut_at(text, MAX_BLOCK_BYTES);
    }
}

/// Cuts one table cell to [`MAX_CELL_CHARS`] characters, marking the cut.
///
/// Cells are counted in characters, not bytes, because a table's column widths are what
/// this cap protects; a block's own cap ([`truncate_text`]) is a byte budget.
fn truncate_cell(text: &mut String) {
    if let Some((at, _)) = text.char_indices().nth(MAX_CELL_CHARS) {
        cut_at(text, at);
    }
}

/// Truncates `text` at or just before byte `end`, on a character boundary, and marks the
/// cut with [`ELLIPSIS`].
fn cut_at(text: &mut String, mut end: usize) {
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push(ELLIPSIS);
}

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
    let mut quote: Vec<String> = Vec::new();
    let mut code: Option<Vec<String>> = None;
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
            flush_quote(&mut blocks, &mut quote);
            blocks.push(table);
            at = next;
            continue;
        }
        let head = raw.trim_start();
        if let Some(body) = quote_body(head) {
            para.flush(&mut blocks);
            quote.push(inline(body));
            continue;
        }
        flush_quote(&mut blocks, &mut quote);
        let trimmed = unquote(head);
        if trimmed.starts_with("```") {
            para.flush(&mut blocks);
            code = Some(Vec::new());
        } else if trimmed.is_empty() {
            para.flush(&mut blocks);
        } else if let Some(kind) = line_rule(trimmed) {
            // `===` or `---` under a text line is a setext heading; on its own either is a
            // rule, as are `***` and `___`. A row of `|`, `-`, and `:` that no table row
            // precedes is a stray separator and is dropped.
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
                    depth: indent_depth(raw),
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
    flush_quote(&mut blocks, &mut quote);
    para.flush(&mut blocks);
    blocks
}

/// How deeply a list item is nested: two leading spaces or one tab per level, capped at
/// [`MAX_BULLET_DEPTH`].
fn indent_depth(line: &str) -> u8 {
    let mut spaces = 0usize;
    let mut tabs = 0usize;
    for c in line.chars() {
        match c {
            ' ' => spaces += 1,
            '\t' => tabs += 1,
            _ => break,
        }
    }
    (tabs + spaces / 2).min(MAX_BULLET_DEPTH as usize) as u8
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

/// Pushes the collected quote lines as one block, if any carry text.
fn flush_quote(blocks: &mut Vec<Block>, lines: &mut Vec<String>) {
    let text = collapse(&std::mem::take(lines).join(" "));
    if !text.is_empty() {
        blocks.push(Block::Quote(text));
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

/// Whether a line is a pipe-table row.
fn is_table_row(line: &str) -> bool {
    line.starts_with('|') || line.contains(" | ")
}

/// Splits one pipe-table row into its cells, inline markup rewritten and each cell cut at
/// [`MAX_CELL_CHARS`].
fn table_cells(line: &str) -> Vec<String> {
    let body = line.strip_prefix('|').unwrap_or(line);
    let body = body.strip_suffix('|').unwrap_or(body);
    body.split('|')
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

/// [`inline_into`] for a caller with nowhere to put images, such as a heading or a table
/// cell: any image the text carried is dropped.
fn inline(text: &str) -> String {
    inline_into(text, &mut Vec::new())
}

/// Rewrites markdown inline markup for display: links flattened to `text (url)`, emphasis
/// and inline-code markers removed, and every `![alt](url)` moved into `images` as a
/// [`Block::Image`], since an image is a block of its own rather than part of the line.
///
/// A badge — an image that is a link's whole label — is dropped outright, image and all.
fn inline_into(text: &str, images: &mut Vec<Block>) -> String {
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

/// Whether `chars` holds `pat` at `at`.
fn starts_with(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(offset, c)| chars.get(at + offset) == Some(&c))
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
    let alt = collapse(&decode_entities(&attr(attrs, "alt"))).replace(['[', ']'], " ");
    format!("\n![{}]({src})\n", collapse(&alt))
}

/// Squeezes every run of whitespace into one space and trims the ends.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out
}

/// Converts HTML to blocks.
///
/// `<p>`, `<h1>`–`<h6>`, `<li>`, `<summary>`, and `<pre>` open a block; `<table>` builds a
/// [`Block::Table`], `<hr>` a [`Block::Rule`], `<blockquote>` a [`Block::Quote`], and
/// `<img src alt>` a [`Block::Image`]; `<ul>`/`<ol>` nesting sets a bullet's depth;
/// `<a href>` becomes `text (href)`; `<br>` becomes a space; `<script>` and `<style>` are
/// dropped with their contents. Every other tag is stripped and its text kept.
pub fn from_html(text: &str) -> Vec<Block> {
    let mut state = HtmlState::default();
    for token in tokenize(text) {
        state.apply(token);
    }
    state.finish_table();
    state.flush();
    capped(state.blocks)
}

/// What the current run of text will become.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Paragraph,
    Heading(u8),
    /// A list item and how deeply its list is nested.
    Bullet(u8),
    Code,
}

/// The table [`HtmlState`] is inside, if any.
#[derive(Debug, Default)]
struct TableBuild {
    /// Open `<table>` tags: a nested table is folded into the outer one.
    depth: u32,
    /// Every finished row and whether it was written with `<th>` cells.
    rows: Vec<(bool, Vec<String>)>,
    /// The row being filled.
    row: Vec<String>,
    /// Whether a `<td>` or `<th>` is open, so its text is the current buffer.
    cell: bool,
    /// Whether the row being filled has a `<th>` in it.
    header_row: bool,
}

/// Builder for [`from_html`]: one block at a time, plus the skip, link, and table state.
#[derive(Debug, Default)]
struct HtmlState {
    blocks: Vec<Block>,
    buf: String,
    mode: Mode,
    /// Tag whose whole subtree is dropped, and how deep we are inside it.
    skip: Option<(String, u32)>,
    /// `href` of the open `<a>` and the buffer length when it opened. The URL is appended
    /// when the anchor closes, and only when it produced text of its own.
    anchor: Option<(String, usize)>,
    /// `<br>` tags seen in a row. Two of them end the paragraph.
    breaks: u32,
    /// Open `<ul>`/`<ol>` tags, which set a `<li>`'s depth.
    list_depth: u32,
    /// Open `<blockquote>` tags: prose inside one is a quote.
    quote_depth: u32,
    /// The table being built, if any.
    table: Option<TableBuild>,
}

/// Tags whose contents never reach a block.
const SKIPPED: [&str; 2] = ["script", "style"];

/// The same, on the markdown path, where a `<table>` is dropped rather than built: a
/// markdown body's tables are badge layout, and its real tables are written with pipes.
const SKIPPED_IN_MARKDOWN: [&str; 3] = ["table", "script", "style"];

impl HtmlState {
    fn apply(&mut self, token: Token) {
        if let Some((tag, depth)) = self.skip.as_mut() {
            if let Token::Tag { name, close, .. } = &token
                && name == tag
            {
                if *close {
                    *depth -= 1;
                    if *depth == 0 {
                        self.skip = None;
                    }
                } else {
                    *depth += 1;
                }
            }
            return;
        }
        match token {
            Token::Text(text) => {
                if text.chars().any(|c| !c.is_whitespace()) {
                    self.breaks = 0;
                }
                self.push_text(&text);
            }
            Token::Tag {
                name,
                close,
                self_closing,
                attrs,
            } => self.tag(&name, close, self_closing, &attrs),
        }
    }

    fn push_text(&mut self, text: &str) {
        let decoded = decode_entities(text);
        if self.mode == Mode::Code {
            self.buf.push_str(&decoded);
            return;
        }
        for c in decoded.chars() {
            if c.is_whitespace() {
                if !self.buf.is_empty() && !self.buf.ends_with(' ') {
                    self.buf.push(' ');
                }
            } else {
                self.buf.push(c);
            }
        }
    }

    fn tag(&mut self, name: &str, close: bool, self_closing: bool, attrs: &str) {
        if SKIPPED.contains(&name) {
            // `<table/>` opens nothing, so it must not swallow everything after it.
            if !close && !self_closing {
                self.flush();
                self.skip = Some((name.to_string(), 1));
            }
            return;
        }
        if name != "br" {
            self.breaks = 0;
        }
        match name {
            "table" => self.table_tag(close, self_closing),
            "tr" => match self.table.is_some() {
                true => {
                    self.end_cell();
                    self.end_row();
                }
                false => self.flush(),
            },
            "td" | "th" => match self.table.is_some() {
                true => {
                    self.end_cell();
                    if !close {
                        self.buf.clear();
                        if let Some(table) = self.table.as_mut() {
                            table.cell = true;
                            table.header_row |= name == "th";
                        }
                    }
                }
                false => self.flush(),
            },
            "img" => {
                let src = decode_entities(&attr(attrs, "src"));
                let src = src.trim().to_string();
                if !close && !src.is_empty() && self.table.is_none() {
                    self.flush();
                    let alt = collapse(&decode_entities(&attr(attrs, "alt")));
                    self.blocks.push(Block::Image { url: src, alt });
                }
            }
            "hr" => {
                if self.table.is_none() {
                    self.flush();
                    self.blocks.push(Block::Rule);
                }
            }
            "blockquote" => {
                self.flush();
                self.quote_depth = match (close, self_closing) {
                    (true, _) => self.quote_depth.saturating_sub(1),
                    (false, false) => self.quote_depth + 1,
                    (false, true) => self.quote_depth,
                };
            }
            "ul" | "ol" => {
                self.flush();
                self.list_depth = match (close, self_closing) {
                    (true, _) => self.list_depth.saturating_sub(1),
                    (false, false) => self.list_depth + 1,
                    (false, true) => self.list_depth,
                };
            }
            "br" => {
                self.breaks += 1;
                if self.breaks >= 2 {
                    self.flush();
                } else {
                    self.push_text(" ");
                }
            }
            "a" => {
                if close {
                    if let Some((href, start)) = self.anchor.take()
                        && !href.is_empty()
                        && self.buf.len() > start
                    {
                        self.buf.push_str(&format!(" ({href})"));
                    }
                } else {
                    self.anchor = Some((decode_entities(&attr(attrs, "href")), self.buf.len()));
                }
            }
            "p" | "div" => self.flush(),
            "li" => {
                self.flush();
                if !close {
                    let depth = self
                        .list_depth
                        .saturating_sub(1)
                        .min(MAX_BULLET_DEPTH.into());
                    self.mode = Mode::Bullet(depth as u8);
                }
            }
            "pre" => {
                self.flush();
                if !close {
                    self.mode = Mode::Code;
                }
            }
            "summary" => {
                self.flush();
                if !close {
                    self.mode = Mode::Heading(3);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                if !close {
                    let level = name[1..].parse::<u8>().unwrap_or(1).clamp(1, 6);
                    self.mode = Mode::Heading(level);
                }
            }
            _ => {}
        }
    }

    /// Opens, nests, or closes a `<table>`. A self-closing one opens nothing, so it cannot
    /// swallow the rest of the document.
    fn table_tag(&mut self, close: bool, self_closing: bool) {
        match (close, self.table.as_mut()) {
            (false, Some(table)) => table.depth += 1,
            (false, None) if !self_closing => {
                self.flush();
                self.table = Some(TableBuild {
                    depth: 1,
                    ..TableBuild::default()
                });
            }
            (false, None) => {}
            (true, Some(table)) => {
                table.depth -= 1;
                if table.depth == 0 {
                    self.finish_table();
                }
            }
            (true, None) => {}
        }
    }

    /// Ends the open `<td>`/`<th>`, moving the buffered text into the current row.
    fn end_cell(&mut self) {
        if !self.table.as_ref().is_some_and(|table| table.cell) {
            return;
        }
        let mut text = collapse(&std::mem::take(&mut self.buf));
        truncate_cell(&mut text);
        if let Some(table) = self.table.as_mut() {
            table.cell = false;
            if table.row.len() < MAX_TABLE_COLS {
                table.row.push(text);
            }
        }
    }

    /// Ends the open `<tr>`, keeping the row it filled.
    fn end_row(&mut self) {
        if let Some(table) = self.table.as_mut()
            && !table.row.is_empty()
        {
            let row = std::mem::take(&mut table.row);
            let header = std::mem::take(&mut table.header_row);
            table.rows.push((header, row));
        }
    }

    /// Closes the table being built and pushes it as one [`Block::Table`].
    ///
    /// The header is the first row written with `<th>` cells, or the first row when none
    /// was. Body rows past [`MAX_TABLE_ROWS`] are dropped and every row is squared off to
    /// the header's width. A table with no cells at all produces no block.
    fn finish_table(&mut self) {
        self.end_cell();
        self.end_row();
        let Some(table) = self.table.take() else {
            return;
        };
        let mut rows = table.rows;
        if rows.is_empty() {
            return;
        }
        let at = rows.iter().position(|(header, _)| *header).unwrap_or(0);
        let header = rows.remove(at).1;
        rows.truncate(MAX_TABLE_ROWS);
        let rows = rows
            .into_iter()
            .map(|(_, mut row)| {
                row.resize(header.len(), String::new());
                row
            })
            .collect();
        self.blocks.push(Block::Table { header, rows });
    }

    /// Ends the current block, dropping it when it holds no text. While a table is open the
    /// buffer is a cell's text, so nothing is flushed out of it.
    fn flush(&mut self) {
        if self.table.is_some() {
            return;
        }
        let text = std::mem::take(&mut self.buf);
        let mode = std::mem::take(&mut self.mode);
        let text = match mode {
            Mode::Code => text.trim_matches('\n').to_string(),
            _ => text.trim().to_string(),
        };
        if text.trim().is_empty() {
            return;
        }
        self.blocks.push(match mode {
            Mode::Paragraph if self.quote_depth > 0 => Block::Quote(text),
            Mode::Paragraph => Block::Paragraph(text),
            Mode::Heading(level) => Block::Heading(level, text),
            Mode::Bullet(depth) => Block::Bullet { depth, text },
            Mode::Code => Block::Code(text),
        });
    }
}

/// One piece of an HTML document: a text run or a tag.
#[derive(Debug)]
enum Token {
    Text(String),
    Tag {
        name: String,
        close: bool,
        /// `<br/>` and friends: the tag opens no subtree.
        self_closing: bool,
        attrs: String,
    },
}

/// Splits HTML into text runs and tags. Comments and doctypes are dropped.
fn tokenize(html: &str) -> Vec<Token> {
    let chars: Vec<char> = html.chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '<' {
            text.push(chars[i]);
            i += 1;
            continue;
        }
        if starts_with(&chars, i, "<!--") {
            i = find_from(&chars, i, "-->").map_or(chars.len(), |end| end + 3);
            continue;
        }
        let Some(end) = tag_end(&chars, i) else {
            // A stray `<` with no `>` after it is text, not a tag.
            text.push(chars[i]);
            i += 1;
            continue;
        };
        if !text.is_empty() {
            out.push(Token::Text(std::mem::take(&mut text)));
        }
        let inner: String = chars[i + 1..end].iter().collect();
        if !inner.starts_with('!') {
            out.push(parse_tag(&inner));
        }
        i = end + 1;
    }
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    out
}

/// Index of the `>` closing the tag that starts at `at`, quotes respected.
fn tag_end(chars: &[char], at: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in chars.iter().enumerate().skip(at + 1) {
        match quote {
            Some(q) if *c == q => quote = None,
            Some(_) => {}
            None if *c == '"' || *c == '\'' => quote = Some(*c),
            None if *c == '>' => return Some(i),
            None if *c == '<' => return None,
            None => {}
        }
    }
    None
}

/// Index of the first `pat` at or after `at`.
fn find_from(chars: &[char], at: usize, pat: &str) -> Option<usize> {
    (at..chars.len()).find(|i| starts_with(chars, *i, pat))
}

/// Splits a tag's inner text into its name, its attributes, and whether it closes itself.
fn parse_tag(inner: &str) -> Token {
    let inner = inner.trim();
    let self_closing = inner.ends_with('/');
    let inner = inner.trim_end_matches('/').trim_end();
    let close = inner.starts_with('/');
    let body = inner.trim_start_matches('/').trim_start();
    let split = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
    Token::Tag {
        name: body[..split].to_ascii_lowercase(),
        close,
        self_closing,
        attrs: body[split..].trim().to_string(),
    }
}

/// The value of `key` in an attribute string, quoted or bare, or an empty string.
///
/// The string is read as name/value pairs, so a value that mentions `key` (a `title` with
/// `href=` in it, say) is never mistaken for the attribute itself.
fn attr(attrs: &str, key: &str) -> String {
    for (name, value) in attr_pairs(attrs) {
        if name.eq_ignore_ascii_case(key) {
            return value;
        }
    }
    String::new()
}

/// Every `name="value"` pair in an attribute string, quotes respected.
fn attr_pairs(attrs: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = attrs.chars().collect();
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() || chars[i] == '=' {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if chars.get(i) != Some(&'=') {
            // A valueless attribute, such as `disabled`.
            pairs.push((name, String::new()));
            continue;
        }
        i += 1;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let value = match chars.get(i) {
            Some(quote @ ('"' | '\'')) => {
                let quote = *quote;
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                let value: String = chars[start..i].iter().collect();
                i += 1;
                value
            }
            _ => {
                let start = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[start..i].iter().collect()
            }
        };
        pairs.push((name, value.trim().to_string()));
    }
    pairs
}

/// Longest entity this decoder will look at, `&` and `;` included.
const MAX_ENTITY: usize = 12;

/// Replaces the HTML entities a description actually carries: numeric ones and a short
/// named list. Anything else is left as it was written.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '&' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let end = (i + 1..chars.len().min(i + MAX_ENTITY)).find(|j| chars[*j] == ';');
        match end.and_then(|end| entity(&chars[i + 1..end])) {
            Some(text) => {
                out.push_str(&text);
                i = end.unwrap_or(i) + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// The text an entity body (between `&` and `;`) stands for, or `None`.
fn entity(body: &[char]) -> Option<String> {
    let name: String = body.iter().collect();
    if let Some(number) = name.strip_prefix('#') {
        let value = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(value).map(String::from);
    }
    let text = match name.to_ascii_lowercase().as_str() {
        "nbsp" => " ",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "amp" => "&",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "rsquo" => "’",
        "lsquo" => "‘",
        "ldquo" => "“",
        "rdquo" => "”",
        "middot" => "·",
        "copy" => "©",
        "trade" => "™",
        "reg" => "®",
        _ => return None,
    };
    Some(text.to_string())
}
