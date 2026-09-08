//! Tests for the rich-text converters. The markdown input is the `body` of
//! `tests/fixtures/modrinth/project_description.json`; the HTML input is the `data`
//! string of `tests/fixtures/curseforge/get_mod_description.json`. Both cover a
//! heading, a paragraph, a bulleted list, a fenced code block, and a link, and the
//! HTML one also carries an `<img>` and a `<table>`, which become an [`Block::Image`] and
//! a [`Block::Table`].

use super::*;

pub(super) const PROJECT: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_description.json");
pub(super) const DESCRIPTION: &str =
    include_str!("../../../../../tests/fixtures/curseforge/get_mod_description.json");

pub(super) const IMMEDIATELYFAST: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_body_immediatelyfast.json");
pub(super) const MODMENU: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_body_modmenu.json");
pub(super) const SODIUM: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_body_sodium.json");
pub(super) const CREATE: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_body_create.json");
pub(super) const JEI: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_body_jei.json");

/// A depth-`depth` bullet, the shape most cases here use.
pub(super) fn bullet(depth: u8, text: &str) -> Block {
    Block::Bullet {
        depth,
        text: text.to_string(),
    }
}

/// Every string a block carries that markup must not survive in: its table cells
/// included, its code left out, since a code block keeps its source verbatim and a Java
/// generic (`Consumer<String>`, in Mod Menu's body) is not a tag.
pub(super) fn strings(block: &Block) -> Vec<&str> {
    match block {
        Block::Code(_) => Vec::new(),
        Block::Table { header, rows } => header
            .iter()
            .chain(rows.iter().flatten())
            .map(String::as_str)
            .collect(),
        Block::Rule => Vec::new(),
        other => vec![other.text()],
    }
}

/// Pulls a string field out of a fixture without going through a source client.
pub(super) fn field(json: &str, key: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(json).expect("fixture parses");
    value[key].as_str().expect("field is a string").to_string()
}

#[test]
fn empty_input_has_no_blocks() {
    assert!(from_markdown("").is_empty());
    assert!(from_markdown("   \n\n  ").is_empty());
    assert!(from_html("").is_empty());
    assert!(from_html("<p></p><ul></ul>").is_empty());
}

#[test]
fn two_line_breaks_end_the_paragraph() {
    assert_eq!(
        from_html("<p>one<br><br>two</p>"),
        vec![
            Block::Paragraph("one".to_string()),
            Block::Paragraph("two".to_string()),
        ]
    );
    assert_eq!(
        from_markdown("one<br><br>two"),
        vec![
            Block::Paragraph("one".to_string()),
            Block::Paragraph("two".to_string()),
        ]
    );
}

#[test]
fn block_text_answers_for_every_kind() {
    assert_eq!(Block::Rule.text(), "");
    assert_eq!(
        Block::Table {
            header: vec!["A".to_string()],
            rows: vec![vec!["1".to_string()]],
        }
        .text(),
        ""
    );
    assert_eq!(
        Block::Image {
            url: "https://e.test/a.png".to_string(),
            alt: "shot".to_string(),
        }
        .text(),
        "shot"
    );
    assert_eq!(Block::Quote("q".to_string()).text(), "q");
    assert_eq!(
        Block::Bullet {
            depth: 2,
            text: "b".to_string(),
        }
        .text(),
        "b"
    );
}

#[test]
fn a_capped_block_fits_inside_the_byte_cap_with_its_ellipsis() {
    let blocks = from_markdown(&"word ".repeat(4_000));
    assert_eq!(blocks.len(), 1);
    let text = blocks[0].text();
    assert!(text.len() <= MAX_BLOCK_BYTES, "{}", text.len());
    assert!(text.ends_with(ELLIPSIS));
}
