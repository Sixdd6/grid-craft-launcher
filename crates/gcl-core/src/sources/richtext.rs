//! Project descriptions as a short list of display blocks.
//!
//! Modrinth serves a description as markdown (`body` on the project) and CurseForge
//! serves it as HTML (`GET /v1/mods/{id}/description`). Both become the same
//! [`Block`] list so the UI renders one shape. This is a display converter, not a
//! markdown or HTML implementation: it keeps headings, paragraphs, bullets, code
//! blocks, and link URLs, and drops everything else, images and tables included.
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
    /// One list item, from any list depth.
    Bullet(String),
    /// A code block, newlines kept.
    Code(String),
}

impl Block {
    /// The block's text, without its kind.
    pub fn text(&self) -> &str {
        match self {
            Block::Heading(_, text)
            | Block::Paragraph(text)
            | Block::Bullet(text)
            | Block::Code(text) => text,
        }
    }

    /// The block's text for editing, without its kind.
    fn text_mut(&mut self) -> &mut String {
        match self {
            Block::Heading(_, text)
            | Block::Paragraph(text)
            | Block::Bullet(text)
            | Block::Code(text) => text,
        }
    }
}

/// Most blocks one description may produce.
const MAX_BLOCKS: usize = 2_000;

/// Most bytes one block's text may hold.
const MAX_BLOCK_BYTES: usize = 8 * 1024;

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
        truncate_text(block.text_mut());
    }
    if over {
        blocks.push(Block::Paragraph(ELLIPSIS.to_string()));
    }
    blocks
}

/// Cuts `text` to [`MAX_BLOCK_BYTES`] on a character boundary, marking the cut.
fn truncate_text(text: &mut String) {
    if text.len() <= MAX_BLOCK_BYTES {
        return;
    }
    let mut end = MAX_BLOCK_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push(ELLIPSIS);
}

/// Converts markdown to blocks.
///
/// Headings come from leading `#` or a `===`/`---` underline, bullets from a leading
/// `-`, `*`, `+`, `1.`, or `1)`, code from a ``` ``` ``` fence; every other non-empty run
/// of lines is one paragraph, joined with spaces. `[text](url)` becomes `text (url)`,
/// `![alt](url)` is dropped, and `**`, `__`, and backtick markers are removed. A single
/// `*` or `_` is left alone so a `snake_case` word survives. A leading `> ` is dropped,
/// as are `---` rules and table separator rows.
///
/// A real body carries HTML too — `<center>`, `<details>`, badge tables, `<br>`. Tags
/// outside a code fence are stripped first ([`strip_tags`]), and a body that is block-level
/// HTML with no markdown structure left in it goes to [`from_html`] whole.
pub fn from_markdown(text: &str) -> Vec<Block> {
    if is_block_html(text) {
        return from_html(text);
    }
    capped(scan_markdown(&strip_tags_outside_fences(text)))
}

/// The line scanner behind [`from_markdown`], over text whose tags are already stripped.
fn scan_markdown(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut code: Option<Vec<String>> = None;

    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(lines) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                push_code(&mut blocks, std::mem::take(lines));
                code = None;
            } else {
                lines.push(line.to_string());
            }
            continue;
        }
        let trimmed = unquote(line.trim_start());
        if trimmed.starts_with("```") {
            flush_paragraph(&mut blocks, &mut paragraph);
            code = Some(Vec::new());
        } else if trimmed.is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
        } else if let Some(level) = rule_line(trimmed) {
            // `===` or `---` under a text line is a heading; on its own it is a rule, and a
            // row of `|`, `-`, and `:` is a table separator. Both are dropped.
            let text = paragraph.join(" ").trim().to_string();
            paragraph.clear();
            match (level, text.is_empty()) {
                (Some(level), false) => blocks.push(Block::Heading(level, collapse(&text))),
                (_, false) => blocks.push(Block::Paragraph(collapse(&text))),
                _ => {}
            }
        } else if let Some(rest) = trimmed.strip_prefix('#') {
            flush_paragraph(&mut blocks, &mut paragraph);
            let extra = rest.chars().take_while(|c| *c == '#').count();
            let level = (1 + extra).min(6) as u8;
            let body = inline(rest.trim_start_matches('#').trim());
            if !body.is_empty() {
                blocks.push(Block::Heading(level, body));
            }
        } else if let Some(rest) = bullet_body(trimmed) {
            flush_paragraph(&mut blocks, &mut paragraph);
            let body = inline(rest.trim());
            if !body.is_empty() {
                blocks.push(Block::Bullet(body));
            }
        } else {
            paragraph.push(inline(trimmed));
        }
    }
    if let Some(lines) = code {
        push_code(&mut blocks, lines);
    }
    flush_paragraph(&mut blocks, &mut paragraph);
    blocks
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

/// Whether a line is a rule, a table separator, or a setext underline, and which heading
/// level it would give the text line above it.
///
/// `Some(Some(1))` for `===`, `Some(Some(2))` for `---`, `Some(None)` for a table separator
/// row (only `|`, `-`, and `:` in it), and `None` for an ordinary line.
fn rule_line(line: &str) -> Option<Option<u8>> {
    let body: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    if body.len() < 2 {
        return None;
    }
    if body.chars().all(|c| c == '=') {
        return Some(Some(1));
    }
    if body.chars().all(|c| c == '-') {
        return Some(Some(2));
    }
    if body.contains('|') && body.chars().all(|c| matches!(c, '|' | '-' | ':')) {
        return Some(None);
    }
    None
}

/// Pushes the collected paragraph lines as one block, if any carry text.
fn flush_paragraph(blocks: &mut Vec<Block>, lines: &mut Vec<String>) {
    let text = lines.join(" ").trim().to_string();
    lines.clear();
    if !text.is_empty() {
        blocks.push(Block::Paragraph(collapse(&text)));
    }
}

/// Pushes the collected fence lines as one code block, if any carry text.
fn push_code(blocks: &mut Vec<Block>, lines: Vec<String>) {
    let text = lines.join("\n").trim_matches('\n').to_string();
    if !text.trim().is_empty() {
        blocks.push(Block::Code(text));
    }
}

/// Rewrites markdown inline markup for display: images out, links flattened to
/// `text (url)`, emphasis and inline-code markers removed.
fn inline(text: &str) -> String {
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
        if chars[i] == '`' {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    collapse(&out)
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

/// Drops every tag from `html` and keeps its text, entities decoded.
///
/// The same tokenizer [`from_html`] uses: `<img>` and the subtree of a [`SKIPPED`] tag are
/// dropped, a block-level tag becomes a line break, a `<li>` becomes a `-` bullet marker,
/// and two `<br>` in a row become a blank line so the paragraph ends there.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut skip: Option<(String, u32)> = None;
    let mut breaks = 0u32;
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
                ..
            } => {
                if SKIPPED.contains(&name.as_str()) {
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
                    "img" => {}
                    "li" if !close => out.push_str("\n- "),
                    "li" | "tr" | "hr" => out.push('\n'),
                    name if BLOCK_TAGS.contains(&name) => out.push('\n'),
                    _ => {}
                }
            }
        }
    }
    out
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
/// `<p>`, `<h1>`–`<h6>`, `<li>`, and `<pre>` open a block; `<a href>` becomes
/// `text (href)`; `<br>` becomes a space; `<img>`, `<table>`, `<script>`, and
/// `<style>` are dropped with their contents. Every other tag is stripped and its
/// text kept.
pub fn from_html(text: &str) -> Vec<Block> {
    let mut state = HtmlState::default();
    for token in tokenize(text) {
        state.apply(token);
    }
    state.flush();
    capped(state.blocks)
}

/// What the current run of text will become.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Paragraph,
    Heading(u8),
    Bullet,
    Code,
}

/// Builder for [`from_html`]: one block at a time, plus the skip and link state.
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
}

/// Tags whose contents never reach a block.
const SKIPPED: [&str; 3] = ["table", "script", "style"];

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
            "img" => {}
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
            "p" | "div" | "blockquote" | "ul" | "ol" | "tr" | "td" | "th" => self.flush(),
            "li" => {
                self.flush();
                if !close {
                    self.mode = Mode::Bullet;
                }
            }
            "pre" => {
                self.flush();
                if !close {
                    self.mode = Mode::Code;
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

    /// Ends the current block, dropping it when it holds no text.
    fn flush(&mut self) {
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
            Mode::Paragraph => Block::Paragraph(text),
            Mode::Heading(level) => Block::Heading(level, text),
            Mode::Bullet => Block::Bullet(text),
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
