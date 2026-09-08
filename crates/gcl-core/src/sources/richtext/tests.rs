//! Tests for the rich-text converters. The markdown input is the `body` of
//! `tests/fixtures/modrinth/project_description.json`; the HTML input is the `data`
//! string of `tests/fixtures/curseforge/get_mod_description.json`. Both cover a
//! heading, a paragraph, a bulleted list, a fenced code block, and a link, and the
//! HTML one also carries an `<img>` and a `<table>` that must be dropped.

use super::*;

const PROJECT: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_description.json");
const DESCRIPTION: &str =
    include_str!("../../../../../tests/fixtures/curseforge/get_mod_description.json");

/// Pulls a string field out of a fixture without going through a source client.
fn field(json: &str, key: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(json).expect("fixture parses");
    value[key].as_str().expect("field is a string").to_string()
}

#[test]
fn markdown_fixture_becomes_blocks() {
    let blocks = from_markdown(&field(PROJECT, "body"));
    assert_eq!(
        blocks,
        vec![
            Block::Heading(1, "Example Mod".to_string()),
            Block::Paragraph(
                "Example Mod adds one thing to the game. It works on Fabric and Quilt.".to_string()
            ),
            Block::Bullet("Fast".to_string()),
            Block::Bullet("Small".to_string()),
            Block::Bullet("Docs (https://example.com/docs)".to_string()),
            Block::Code("[example]\nenabled = true".to_string()),
            Block::Heading(2, "Install".to_string()),
            Block::Paragraph("Drop the jar in mods/.".to_string()),
        ]
    );
}

#[test]
fn html_fixture_becomes_blocks_without_image_or_table() {
    let blocks = from_html(&field(DESCRIPTION, "data"));
    assert_eq!(
        blocks,
        vec![
            Block::Heading(1, "Example Mod".to_string()),
            Block::Paragraph(
                "Example Mod adds one thing to the game. It works on Forge.".to_string()
            ),
            Block::Bullet("Fast".to_string()),
            Block::Bullet("Small".to_string()),
            Block::Bullet("Docs (https://example.com/docs)".to_string()),
            Block::Code("[example]\nenabled = true".to_string()),
            Block::Heading(2, "Install".to_string()),
            Block::Paragraph("Drop the jar in mods/ & play.".to_string()),
        ]
    );
    let text = blocks.iter().map(Block::text).collect::<String>();
    assert!(!text.contains("banner"), "image survived: {text}");
    assert!(!text.contains("1.20.1"), "table survived: {text}");
}

#[test]
fn markdown_heading_level_is_clamped_to_six() {
    assert_eq!(
        from_markdown("####### deep"),
        vec![Block::Heading(6, "deep".to_string())]
    );
}

#[test]
fn markdown_star_bullets_and_images_and_bold() {
    assert_eq!(
        from_markdown("* one\n* ![shot](https://e.test/a.png) two"),
        vec![
            Block::Bullet("one".to_string()),
            Block::Bullet("two".to_string()),
        ]
    );
    assert_eq!(
        from_markdown("__all bold__ and `code`"),
        vec![Block::Paragraph("all bold and code".to_string())]
    );
}

#[test]
fn markdown_keeps_snake_case_words_intact() {
    assert_eq!(
        from_markdown("set enable_thing in the config"),
        vec![Block::Paragraph(
            "set enable_thing in the config".to_string()
        )]
    );
}

#[test]
fn unclosed_markdown_fence_still_yields_its_code() {
    assert_eq!(
        from_markdown("```\nline\n"),
        vec![Block::Code("line".to_string())]
    );
}

#[test]
fn empty_input_has_no_blocks() {
    assert!(from_markdown("").is_empty());
    assert!(from_markdown("   \n\n  ").is_empty());
    assert!(from_html("").is_empty());
    assert!(from_html("<p></p><ul></ul>").is_empty());
}

#[test]
fn html_entities_and_nested_inline_tags_are_flattened() {
    assert_eq!(
        from_html("<p>a &lt;b&gt; &quot;c&quot; &#39;d&#39; &nbsp;e</p>"),
        vec![Block::Paragraph("a <b> \"c\" 'd' e".to_string())]
    );
    assert_eq!(
        from_html("<h3>A <em>strong</em> title</h3>"),
        vec![Block::Heading(3, "A strong title".to_string())]
    );
}

#[test]
fn html_text_outside_any_block_tag_becomes_a_paragraph() {
    assert_eq!(
        from_html("loose text<h2>Head</h2>"),
        vec![
            Block::Paragraph("loose text".to_string()),
            Block::Heading(2, "Head".to_string()),
        ]
    );
}

#[test]
fn html_script_and_style_are_dropped() {
    assert_eq!(
        from_html("<style>p{color:red}</style><script>alert(1)</script><p>ok</p>"),
        vec![Block::Paragraph("ok".to_string())]
    );
}

#[test]
fn html_link_with_single_quoted_href_keeps_its_url() {
    assert_eq!(
        from_html("<p>see <a class='x' href='https://e.test/p'>this page</a></p>"),
        vec![Block::Paragraph(
            "see this page (https://e.test/p)".to_string()
        )]
    );
}

#[test]
fn html_anchor_without_href_keeps_only_its_text() {
    assert_eq!(
        from_html("<p>see <a name=\"top\">this</a></p>"),
        vec![Block::Paragraph("see this".to_string())]
    );
}
