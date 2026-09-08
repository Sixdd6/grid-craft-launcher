//! Tests for the HTML converter.

use super::super::tests::*;
use super::*;

#[test]
fn html_fixture_becomes_blocks_with_its_image_and_table() {
    let blocks = from_html(&field(DESCRIPTION, "data"));
    assert_eq!(
        blocks,
        vec![
            Block::Heading(1, "Example Mod".to_string()),
            Block::Paragraph(
                "Example Mod adds one thing to the game. It works on Forge.".to_string()
            ),
            Block::Image {
                url: "https://example.com/banner.png".to_string(),
                alt: "banner".to_string(),
            },
            bullet(0, "Fast"),
            bullet(0, "Small"),
            bullet(0, "Docs (https://example.com/docs)"),
            Block::Code("[example]\nenabled = true".to_string()),
            Block::Table {
                header: vec!["Version".to_string(), "1.20.1".to_string()],
                rows: Vec::new(),
            },
            Block::Heading(2, "Install".to_string()),
            Block::Paragraph("Drop the jar in mods/ & play.".to_string()),
        ]
    );
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
        vec![
            Block::Paragraph("x".to_string()),
            Block::Image {
                url: "https://e.test/i.png".to_string(),
                alt: String::new(),
            },
        ]
    );
}

#[test]
fn html_table_uses_its_header_cells() {
    assert_eq!(
        from_html("<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>"),
        vec![Block::Table {
            header: vec!["A".to_string(), "B".to_string()],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
        }]
    );
}

#[test]
fn html_table_without_header_cells_uses_its_first_row() {
    assert_eq!(
        from_html("<table><tbody><tr><td>A</td></tr><tr><td>1</td></tr></tbody></table>"),
        vec![Block::Table {
            header: vec!["A".to_string()],
            rows: vec![vec!["1".to_string()]],
        }]
    );
}

#[test]
fn html_rule_quote_image_and_list_depth() {
    assert_eq!(
        from_html("<p>a</p><hr><p>b</p>"),
        vec![
            Block::Paragraph("a".to_string()),
            Block::Rule,
            Block::Paragraph("b".to_string()),
        ]
    );
    assert_eq!(
        from_html("<blockquote><p>quoted</p></blockquote>"),
        vec![Block::Quote("quoted".to_string())]
    );
    assert_eq!(
        from_html("<img src=\"https://e.test/a.png\" alt=\"shot\">"),
        vec![Block::Image {
            url: "https://e.test/a.png".to_string(),
            alt: "shot".to_string(),
        }]
    );
    assert_eq!(
        from_html("<ul><li>one<ul><li>two</li></ul></li></ul>"),
        vec![
            Block::Bullet {
                depth: 0,
                text: "one".to_string()
            },
            Block::Bullet {
                depth: 1,
                text: "two".to_string()
            },
        ]
    );
}

// --- Plan 10 fix round -------------------------------------------------------------------

#[test]
fn details_section_figure_and_center_end_the_block() {
    assert_eq!(
        from_html(
            "<p>one</p><details>two</details><section>three</section><figure>four</figure><center>five</center>"
        ),
        vec![
            Block::Paragraph("one".to_string()),
            Block::Paragraph("two".to_string()),
            Block::Paragraph("three".to_string()),
            Block::Paragraph("four".to_string()),
            Block::Paragraph("five".to_string()),
        ]
    );
}

#[test]
fn html_table_rows_are_bounded_while_they_are_collected() {
    let row = "<tr><td>c</td></tr>";
    let html = format!("<table><tr><th>h</th></tr>{}</table>", row.repeat(200));
    match &from_html(&html)[0] {
        Block::Table { header, rows } => {
            assert_eq!(header, &vec!["h".to_string()]);
            assert_eq!(rows.len(), MAX_TABLE_ROWS);
        }
        other => panic!("not a table: {other:?}"),
    }
}
