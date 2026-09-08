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
}

/// Converts markdown to blocks.
///
/// Headings come from leading `#`, bullets from a leading `-` or `*`, code from a
/// ``` ``` ``` fence; every other non-empty run of lines is one paragraph, joined with
/// spaces. `[text](url)` becomes `text (url)`, `![alt](url)` is dropped, and `**`,
/// `__`, and backtick markers are removed. A single `*` or `_` is left alone so a
/// `snake_case` word survives.
pub fn from_markdown(text: &str) -> Vec<Block> {
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
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            flush_paragraph(&mut blocks, &mut paragraph);
            code = Some(Vec::new());
        } else if trimmed.is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
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

/// The text after a `-` or `*` list marker, or `None` when the line is not a bullet.
fn bullet_body(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest);
        }
    }
    match line {
        "-" | "*" | "+" => Some(""),
        _ => None,
    }
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
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '!'
            && let Some(end) = link_at(&chars, i + 1)
        {
            i = end;
            continue;
        }
        if chars[i] == '['
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

/// The index just past a `[label](url)` starting at `at`, or `None`.
fn link_at(chars: &[char], at: usize) -> Option<usize> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let close = (at + 1..chars.len()).find(|i| chars[*i] == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|i| chars[*i] == ')')?;
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
    state.blocks
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
    /// `href` of the open `<a>`, appended when it closes.
    href: Option<String>,
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
            Token::Text(text) => self.push_text(&text),
            Token::Tag { name, close, attrs } => self.tag(&name, close, &attrs),
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

    fn tag(&mut self, name: &str, close: bool, attrs: &str) {
        if SKIPPED.contains(&name) {
            if !close {
                self.flush();
                self.skip = Some((name.to_string(), 1));
            }
            return;
        }
        match name {
            "img" => {}
            "br" => self.push_text(" "),
            "a" => {
                if close {
                    if let Some(href) = self.href.take()
                        && !href.is_empty()
                    {
                        self.buf.push_str(&format!(" ({href})"));
                    }
                } else {
                    self.href = Some(decode_entities(&attr(attrs, "href")));
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

/// Splits a tag's inner text into its name and its attributes.
fn parse_tag(inner: &str) -> Token {
    let inner = inner.trim().trim_end_matches('/');
    let close = inner.starts_with('/');
    let body = inner.trim_start_matches('/').trim_start();
    let split = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
    Token::Tag {
        name: body[..split].to_ascii_lowercase(),
        close,
        attrs: body[split..].trim().to_string(),
    }
}

/// The value of `key` in an attribute string, quoted or bare, or an empty string.
fn attr(attrs: &str, key: &str) -> String {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(key) {
        let start = from + found;
        from = start + key.len();
        let before_ok = start == 0
            || lower[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        let rest = attrs[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let quote = value.chars().next();
        return match quote {
            Some(q @ ('"' | '\'')) => value[1..]
                .split(q)
                .next()
                .unwrap_or_default()
                .trim()
                .to_string(),
            _ => value
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string(),
        };
    }
    String::new()
}

/// Replaces the handful of HTML entities a description actually carries.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = text.to_string();
    for (entity, replacement) in [
        ("&nbsp;", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&#x27;", "'"),
        ("&amp;", "&"),
    ] {
        out = out.replace(entity, replacement);
    }
    out
}
