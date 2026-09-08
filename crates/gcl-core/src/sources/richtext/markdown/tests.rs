//! Tests for the markdown converter.

use super::super::tests::*;
use super::*;

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
            bullet(0, "Fast"),
            bullet(0, "Small"),
            bullet(0, "Docs (https://example.com/docs)"),
            Block::Code("[example]\nenabled = true".to_string()),
            Block::Heading(2, "Install".to_string()),
            Block::Paragraph("Drop the jar in mods/.".to_string()),
        ]
    );
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
            bullet(0, "one"),
            bullet(0, "two"),
            Block::Image {
                url: "https://e.test/a.png".to_string(),
                alt: "shot".to_string(),
            },
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
fn markdown_strips_html_tags_and_keeps_images() {
    let blocks = from_markdown(
        "<center>\n <img src=\"https://e.test/banner.png\" alt=\"banner\"></img>\n</center>\n\n# Head\n\ntext <b>bold</b> &amp; more",
    );
    assert_eq!(
        blocks,
        vec![
            Block::Image {
                url: "https://e.test/banner.png".to_string(),
                alt: "banner".to_string(),
            },
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
        vec![bullet(0, "one"), bullet(0, "two"), bullet(0, "ten")]
    );
}

#[test]
fn a_body_of_block_html_is_delegated_to_the_html_path() {
    assert_eq!(
        from_markdown("<h2>Head</h2><p>prose</p><ul><li>one</li></ul>"),
        vec![
            Block::Heading(2, "Head".to_string()),
            Block::Paragraph("prose".to_string()),
            bullet(0, "one"),
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
fn markdown_table_rows_quotes_and_rules() {
    assert_eq!(
        from_markdown("| :--- | ---: |\n---\n> quoted line\n\nplain"),
        vec![
            Block::Rule,
            Block::Quote("quoted line".to_string()),
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
    for (name, json) in [
        ("sodium", SODIUM),
        ("create", CREATE),
        ("jei", JEI),
        ("immediatelyfast", IMMEDIATELYFAST),
        ("modmenu", MODMENU),
    ] {
        let blocks = from_markdown(&field(json, "body"));
        assert!(!blocks.is_empty(), "{name}: no blocks");
        assert!(blocks.len() < 2_000, "{name}: {} blocks", blocks.len());
        for block in &blocks {
            for text in strings(block) {
                assert!(!has_tag(text), "{name}: html left in {block:?}");
            }
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
            blocks.iter().any(|b| matches!(b, Block::Bullet { .. })),
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

// --- Plan 10: tables, rules, images, quotes, and bullet depth ---------------------------

#[test]
fn markdown_pipe_table_becomes_a_table_block() {
    assert_eq!(
        from_markdown("| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |"),
        vec![Block::Table {
            header: vec!["A".to_string(), "B".to_string()],
            rows: vec![
                vec!["1".to_string(), "2".to_string()],
                vec!["3".to_string(), "4".to_string()],
            ],
        }]
    );
}

#[test]
fn markdown_table_cells_keep_link_text_and_pad_short_rows() {
    assert_eq!(
        from_markdown("| A | B |\n| :-- | --: |\n| [d](https://e.test) |"),
        vec![Block::Table {
            header: vec!["A".to_string(), "B".to_string()],
            rows: vec![vec!["d (https://e.test)".to_string(), String::new()]],
        }]
    );
}

#[test]
fn a_table_separator_with_no_header_above_it_is_dropped() {
    assert_eq!(
        from_markdown("| :--- | ---: |\n\nplain"),
        vec![Block::Paragraph("plain".to_string())]
    );
}

#[test]
fn immediatelyfast_body_carries_its_comparison_table() {
    let blocks = from_markdown(&field(IMMEDIATELYFAST, "body"));
    let table = blocks
        .iter()
        .find_map(|b| match b {
            Block::Table { header, rows } => Some((header, rows)),
            _ => None,
        })
        .expect("a table block");
    assert_eq!(
        table.0,
        &vec![
            "Other mods".to_string(),
            "Without ImmediatelyFast".to_string(),
            "With ImmediatelyFast".to_string(),
            "Improvement".to_string(),
        ]
    );
    assert_eq!(
        table.1[0],
        vec![
            "None".to_string(),
            "16 FPS".to_string(),
            "60 FPS".to_string(),
            "3.75x".to_string(),
        ]
    );
}

#[test]
fn markdown_rules_become_rule_blocks() {
    assert_eq!(
        from_markdown("one\n\n---\n\ntwo\n\n***\n\n___"),
        vec![
            Block::Paragraph("one".to_string()),
            Block::Rule,
            Block::Paragraph("two".to_string()),
            Block::Rule,
            Block::Rule,
        ]
    );
}

#[test]
fn markdown_quote_lines_become_one_quote_block() {
    assert_eq!(
        from_markdown("> quoted\n> more\n\nplain"),
        vec![
            Block::Quote("quoted more".to_string()),
            Block::Paragraph("plain".to_string()),
        ]
    );
}

#[test]
fn a_nested_markdown_quote_marker_still_reads_as_text() {
    assert_eq!(
        from_markdown(">> deep"),
        vec![Block::Paragraph("deep".to_string())]
    );
}

#[test]
fn markdown_bullet_indentation_becomes_depth() {
    assert_eq!(
        from_markdown("- one\n  - two\n    - three\n\t- tabbed"),
        vec![
            Block::Bullet {
                depth: 0,
                text: "one".to_string()
            },
            Block::Bullet {
                depth: 1,
                text: "two".to_string()
            },
            Block::Bullet {
                depth: 2,
                text: "three".to_string()
            },
            Block::Bullet {
                depth: 1,
                text: "tabbed".to_string()
            },
        ]
    );
}

#[test]
fn markdown_bold_only_paragraph_becomes_a_subheading() {
    assert_eq!(
        from_markdown("**Only bold**\n\n__also bold__\n\n**bold** and more"),
        vec![
            Block::Heading(4, "Only bold".to_string()),
            Block::Heading(4, "also bold".to_string()),
            Block::Paragraph("bold and more".to_string()),
        ]
    );
}

#[test]
fn markdown_images_become_image_blocks() {
    assert_eq!(
        from_markdown("![shot](https://e.test/a.png)"),
        vec![Block::Image {
            url: "https://e.test/a.png".to_string(),
            alt: "shot".to_string(),
        }]
    );
    assert_eq!(
        from_markdown("text ![](https://e.test/b.png) after"),
        vec![
            Block::Paragraph("text after".to_string()),
            Block::Image {
                url: "https://e.test/b.png".to_string(),
                alt: String::new(),
            },
        ]
    );
}

#[test]
fn an_html_image_inside_markdown_becomes_an_image_block() {
    assert_eq!(
        from_markdown(
            "<center>\n <img src=\"https://e.test/banner.png\" alt=\"banner\">\n</center>"
        ),
        vec![Block::Image {
            url: "https://e.test/banner.png".to_string(),
            alt: "banner".to_string(),
        }]
    );
}

#[test]
fn markdown_details_becomes_a_heading_and_its_own_blocks() {
    assert_eq!(
        from_markdown(
            "<details>\n<summary>More detail</summary>\n\nbody text\n\n- one\n\n</details>"
        ),
        vec![
            Block::Heading(3, "More detail".to_string()),
            Block::Paragraph("body text".to_string()),
            Block::Bullet {
                depth: 0,
                text: "one".to_string()
            },
        ]
    );
}

#[test]
fn modmenu_body_keeps_its_details_summary_as_a_heading() {
    let blocks = from_markdown(&field(MODMENU, "body"));
    assert!(
        blocks.contains(&Block::Heading(
            3,
            "Translation API Documentation".to_string()
        )),
        "no summary heading in {:?}",
        blocks
            .iter()
            .filter(|b| matches!(b, Block::Heading(..)))
            .collect::<Vec<_>>()
    );
    assert!(
        blocks.iter().any(|b| matches!(b, Block::Quote(_))),
        "no quote block"
    );
}

#[test]
fn a_table_is_truncated_to_fifty_rows_and_eight_columns() {
    let header = format!(
        "|{}|\n",
        (0..10)
            .map(|i| format!(" h{i} "))
            .collect::<Vec<_>>()
            .join("|")
    );
    let sep = format!("|{}|\n", ["---"; 10].join("|"));
    let row = format!(
        "|{}|\n",
        (0..10)
            .map(|i| format!(" c{i} "))
            .collect::<Vec<_>>()
            .join("|")
    );
    let blocks = from_markdown(&format!("{header}{sep}{}", row.repeat(60)));
    match &blocks[0] {
        Block::Table { header, rows } => {
            assert_eq!(header.len(), 8);
            assert_eq!(rows.len(), 50);
            assert!(rows.iter().all(|r| r.len() == 8));
        }
        other => panic!("not a table: {other:?}"),
    }
}

#[test]
fn a_table_cell_is_cut_at_two_hundred_characters() {
    let long = "x".repeat(300);
    let blocks = from_markdown(&format!("| A |\n|---|\n| {long} |"));
    match &blocks[0] {
        Block::Table { rows, .. } => {
            assert_eq!(rows[0][0].chars().count(), 201);
            assert!(rows[0][0].ends_with('…'));
        }
        other => panic!("not a table: {other:?}"),
    }
}

// --- Plan 10 fix round -------------------------------------------------------------------

#[test]
fn a_table_cell_keeps_an_escaped_pipe_as_text() {
    assert_eq!(
        from_markdown("| A | B |\n|---|---|\n| a \\| b | c |"),
        vec![Block::Table {
            header: vec!["A".to_string(), "B".to_string()],
            rows: vec![vec!["a | b".to_string(), "c".to_string()]],
        }]
    );
}

#[test]
fn a_table_needs_no_outer_pipes_or_spaces() {
    assert_eq!(
        from_markdown("a|b\n-|-\n1|2"),
        vec![Block::Table {
            header: vec!["a".to_string(), "b".to_string()],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
        }]
    );
}

#[test]
fn an_html_image_source_with_parentheses_survives_the_round_trip() {
    // A markdown heading keeps the body on the markdown path, where the `<img>` is
    // rewritten as `![alt](src)` and read back by the line scanner.
    assert_eq!(
        from_markdown("# Title\n\n<img src=\"https://e.test/a(1).png\" alt=\"shot\">"),
        vec![
            Block::Heading(1, "Title".to_string()),
            Block::Image {
                url: "https://e.test/a%281%29.png".to_string(),
                alt: "shot".to_string(),
            },
        ]
    );
}

#[test]
fn a_four_space_indented_list_nests_one_level() {
    assert_eq!(
        from_markdown("- one\n    - two\n        - three"),
        vec![bullet(0, "one"), bullet(1, "two"), bullet(2, "three"),]
    );
}

#[test]
fn a_blank_line_ends_a_quote() {
    assert_eq!(
        from_markdown("> one\n\n> two"),
        vec![
            Block::Quote("one".to_string()),
            Block::Quote("two".to_string()),
        ]
    );
}

#[test]
fn an_image_on_a_quote_line_is_kept() {
    assert_eq!(
        from_markdown("> look ![shot](https://e.test/a.png)"),
        vec![
            Block::Quote("look".to_string()),
            Block::Image {
                url: "https://e.test/a.png".to_string(),
                alt: "shot".to_string(),
            },
        ]
    );
}
