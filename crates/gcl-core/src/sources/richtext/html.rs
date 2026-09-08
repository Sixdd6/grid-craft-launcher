//! The HTML side of [`super`]: a tag-driven block builder.

use super::tokenize::{Token, attr, decode_entities, tokenize};
use super::{
    Block, MAX_BULLET_DEPTH, MAX_TABLE_COLS, MAX_TABLE_ROWS, capped, collapse, truncate_cell,
};

#[cfg(test)]
mod tests;

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
pub(super) const SKIPPED_IN_MARKDOWN: [&str; 3] = ["table", "script", "style"];

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
            // A wrapper that opens or closes a block of its own: the run of text before
            // it ends there, rather than running into the next one.
            "p" | "div" | "details" | "section" | "figure" | "center" => self.flush(),
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
    ///
    /// Rows past [`MAX_TABLE_ROWS`] plus the header are dropped here rather than at
    /// [`HtmlState::finish_table`], so a document with a hundred thousand `<tr>` in it
    /// costs a bounded amount of memory while it is read.
    fn end_row(&mut self) {
        if let Some(table) = self.table.as_mut()
            && !table.row.is_empty()
        {
            let row = std::mem::take(&mut table.row);
            let header = std::mem::take(&mut table.header_row);
            if table.rows.len() <= MAX_TABLE_ROWS {
                table.rows.push((header, row));
            }
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
