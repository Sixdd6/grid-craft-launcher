//! Tests for the HTML tokenizer.

use super::*;

#[test]
fn entities_are_decoded_by_number_and_by_name() {
    assert_eq!(
        decode_entities("a &amp; b &#65; &#x42; &nope;"),
        "a & b A B &nope;"
    );
}

#[test]
fn an_attribute_is_read_by_name_not_by_a_value_that_mentions_it() {
    assert_eq!(
        attr("title=\"href=x\" href='https://e.test'", "href"),
        "https://e.test"
    );
    assert_eq!(attr("disabled src=a.png", "src"), "a.png");
}
