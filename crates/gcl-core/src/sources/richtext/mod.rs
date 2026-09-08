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

mod html;
mod inline;
mod markdown;
mod tokenize;

#[cfg(test)]
mod tests;

pub use html::from_html;
pub use markdown::from_markdown;

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
///
/// The ellipsis is spent out of the same budget, so the block that comes back is never
/// longer than the cap the caller was promised.
fn truncate_text(text: &mut String) {
    if text.len() > MAX_BLOCK_BYTES {
        cut_at(text, MAX_BLOCK_BYTES - ELLIPSIS.len_utf8());
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
