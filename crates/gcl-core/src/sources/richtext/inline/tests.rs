//! Tests for the inline markdown rewriter.

use super::*;

#[test]
fn an_image_is_moved_out_of_the_line_it_sat_on() {
    let mut images = Vec::new();
    let text = inline_into("before ![shot](https://e.test/a.png) after", &mut images);
    assert_eq!(text, "before after");
    assert_eq!(
        images,
        vec![Block::Image {
            url: "https://e.test/a.png".to_string(),
            alt: "shot".to_string(),
        }]
    );
}

#[test]
fn a_link_keeps_its_url_and_drops_its_markers() {
    assert_eq!(
        inline("**bold** [docs](https://e.test) `code`"),
        "bold docs (https://e.test) code"
    );
}
