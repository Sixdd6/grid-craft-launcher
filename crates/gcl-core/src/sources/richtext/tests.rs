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
fn markdown_strips_single_asterisk_emphasis_at_a_word_boundary() {
    assert_eq!(
        from_markdown("this is *important* text"),
        vec![Block::Paragraph("this is important text".to_string())]
    );
}

#[test]
fn markdown_strips_single_underscore_emphasis_at_a_word_boundary() {
    assert_eq!(
        from_markdown("this is _important_ text"),
        vec![Block::Paragraph("this is important text".to_string())]
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

// --- Rulings from the plan-9 fix round ------------------------------------------------

const SODIUM: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_body_sodium.json");
const CREATE: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_body_create.json");
const JEI: &str = include_str!("../../../../../tests/fixtures/modrinth/project_body_jei.json");

#[test]
fn markdown_strips_html_tags_and_drops_images() {
    let blocks = from_markdown(
        "<center>\n <img src=\"https://e.test/banner.png\" alt=\"banner\"></img>\n</center>\n\n# Head\n\ntext <b>bold</b> &amp; more",
    );
    assert_eq!(
        blocks,
        vec![
            Block::Heading(1, "Head".to_string()),
            Block::Paragraph("text bold & more".to_string()),
        ]
    );
}

#[test]
fn markdown_drops_skipped_subtrees() {
    let blocks = from_markdown(
        "<table><tr><td>1.20.1</td></tr></table>\n<script>alert(1)</script>\n<style>p{color:red}</style>\n\nkept",
    );
    assert_eq!(blocks, vec![Block::Paragraph("kept".to_string())]);
}

#[test]
fn markdown_keeps_html_inside_a_code_fence() {
    assert_eq!(
        from_markdown("```\n<config enabled=\"true\">\n```"),
        vec![Block::Code("<config enabled=\"true\">".to_string())]
    );
}

#[test]
fn markdown_ordered_lists_become_bullets() {
    assert_eq!(
        from_markdown("1. one\n2) two\n10. ten"),
        vec![
            Block::Bullet("one".to_string()),
            Block::Bullet("two".to_string()),
            Block::Bullet("ten".to_string()),
        ]
    );
}

#[test]
fn a_body_of_block_html_is_delegated_to_the_html_path() {
    assert_eq!(
        from_markdown("<h2>Head</h2><p>prose</p><ul><li>one</li></ul>"),
        vec![
            Block::Heading(2, "Head".to_string()),
            Block::Paragraph("prose".to_string()),
            Block::Bullet("one".to_string()),
        ]
    );
}

#[test]
fn an_unmatched_bracket_run_stays_linear() {
    let text = format!("[a](https://e.test) {}", "[".repeat(100_000));
    let blocks = from_markdown(&text);
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0].text().starts_with("a (https://e.test)"),
        "{:?}",
        blocks[0]
    );
}

#[test]
fn output_is_capped_at_two_thousand_blocks() {
    let text = "- x\n".repeat(2_500);
    let blocks = from_markdown(&text);
    assert_eq!(blocks.len(), 2_001);
    assert_eq!(blocks[2_000], Block::Paragraph("…".to_string()));
}

#[test]
fn a_block_is_capped_at_eight_kibibytes() {
    let blocks = from_markdown(&"word ".repeat(4_000));
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0].text().len() <= 8 * 1024 + 4,
        "{}",
        blocks[0].text().len()
    );
    assert!(blocks[0].text().ends_with('…'));
}

#[test]
fn a_self_closing_skipped_tag_does_not_swallow_the_rest() {
    assert_eq!(
        from_html("<script/><p>ok</p>"),
        vec![Block::Paragraph("ok".to_string())]
    );
    assert_eq!(
        from_html("<table/><style/><p>ok</p>"),
        vec![Block::Paragraph("ok".to_string())]
    );
}

#[test]
fn an_attribute_value_that_mentions_href_is_not_the_href() {
    assert_eq!(
        from_html("<p><a title=\"href=nope\" href=\"https://good.test\">x</a></p>"),
        vec![Block::Paragraph("x (https://good.test)".to_string())]
    );
    assert_eq!(
        from_html("<p><a data-href=\"https://bad.test\" href=\"https://good.test\">x</a></p>"),
        vec![Block::Paragraph("x (https://good.test)".to_string())]
    );
}

#[test]
fn numeric_and_named_entities_are_decoded() {
    assert_eq!(
        from_html(
            "<p>it&#8217;s it&#x2019;s &mdash; &ndash; &hellip; &copy; &reg; &trade; &ldquo;q&rdquo;</p>"
        ),
        vec![Block::Paragraph("it’s it’s — – … © ® ™ “q”".to_string())]
    );
    assert_eq!(
        from_html("<p>a &notanentity; b</p>"),
        vec![Block::Paragraph("a &notanentity; b".to_string())]
    );
}

#[test]
fn an_anchor_with_no_text_keeps_no_url() {
    assert_eq!(
        from_html("<p>x <a href=\"https://e.test\"><img src=\"https://e.test/i.png\"></a></p>"),
        vec![Block::Paragraph("x".to_string())]
    );
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
fn markdown_table_rows_quotes_and_rules() {
    assert_eq!(
        from_markdown("| :--- | ---: |\n---\n> quoted line\n\nplain"),
        vec![
            Block::Paragraph("quoted line".to_string()),
            Block::Paragraph("plain".to_string()),
        ]
    );
}

#[test]
fn markdown_setext_underlines_become_headings() {
    assert_eq!(
        from_markdown("Title\n===\n\nSub\n---\n\nbody"),
        vec![
            Block::Heading(1, "Title".to_string()),
            Block::Heading(2, "Sub".to_string()),
            Block::Paragraph("body".to_string()),
        ]
    );
}

/// Whether a block still carries something tag-shaped: `<` and then a name or a `/`.
///
/// A bare `<` is not enough. Sodium's body says "for MC <1.19" in prose, and that `<` is
/// text, not markup.
fn has_tag(text: &str) -> bool {
    text.match_indices('<')
        .any(|(at, _)| text[at + 1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/'))
}

/// Every recorded body converts to blocks with no HTML left in them.
#[test]
fn recorded_bodies_convert_without_html() {
    for (name, json) in [("sodium", SODIUM), ("create", CREATE), ("jei", JEI)] {
        let blocks = from_markdown(&field(json, "body"));
        assert!(!blocks.is_empty(), "{name}: no blocks");
        assert!(blocks.len() < 2_000, "{name}: {} blocks", blocks.len());
        for block in &blocks {
            assert!(!has_tag(block.text()), "{name}: html left in {block:?}");
            assert!(
                block.text().len() <= 8 * 1024 + 4,
                "{name}: block over the cap"
            );
        }
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Heading(..))),
            "{name}: no heading"
        );
    }
}

/// Sodium's and JEI's bodies carry lists; Create's carries none, so it is not checked here.
#[test]
fn recorded_bodies_keep_their_lists() {
    for (name, json) in [("sodium", SODIUM), ("jei", JEI)] {
        let blocks = from_markdown(&field(json, "body"));
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Bullet(_))),
            "{name}: no bullet"
        );
    }
}

#[test]
fn a_badge_image_wrapped_in_a_link_is_dropped() {
    assert_eq!(
        from_markdown("[![Discord](https://e.test/badge.png)](https://e.test/join) after"),
        vec![Block::Paragraph("after".to_string())]
    );
}
