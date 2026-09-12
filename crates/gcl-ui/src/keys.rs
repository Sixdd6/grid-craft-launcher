//! Keyboard rules, kept pure so they can be tested without a window.
//!
//! Slint delivers key events to the focused element and bubbles what it rejects up to the
//! enclosing `FocusScope`. The scopes in `.slint` decide *where* a key is handled; the two
//! functions here decide *what* it means.

use crate::Screen;

/// The screen a rail shortcut names, or `None` when the key is not one.
///
/// The digits match the numbers printed in the rail, top to bottom.
pub fn key_to_screen(key: &str) -> Option<Screen> {
    match key {
        "1" => Some(Screen::Instances),
        "2" => Some(Screen::Instance),
        "3" => Some(Screen::Browser),
        "4" => Some(Screen::Accounts),
        "5" => Some(Screen::Settings),
        _ => None,
    }
}

/// Where a list selection lands after an arrow key.
///
/// `current` is the chosen row, or -1 while nothing is chosen. `delta` is the step, normally
/// -1 or 1. The result is clamped to the list rather than wrapped, so holding an arrow key
/// stops at an end instead of jumping to the other one. An empty list has no selection, so it
/// answers -1; a first key press on a list with no selection lands on the first row going
/// down and the last row going up.
pub fn move_selection(current: i32, delta: i32, len: i32) -> i32 {
    if len <= 0 {
        return -1;
    }
    if current < 0 {
        return if delta < 0 { len - 1 } else { 0 };
    }
    // `saturating_add` keeps a huge delta from wrapping around before the clamp.
    current.saturating_add(delta).clamp(0, len - 1)
}

/// The `event.text` a Slint `key-pressed` handler sees for `Key.Home`, `Key.End`,
/// `Key.PageUp`, and `Key.PageDown`. These are private-use Unicode code points (the same ones
/// Qt and the other backends use), not the printable characters their names suggest, so a
/// `.slint` scope compares against `Key.Home` directly rather than these constants; they exist
/// here only so `jump`'s own match arms and its tests can name what they mean.
/// See `i-slint-common::key_codes` for the full table this is drawn from.
pub const KEY_HOME: &str = "\u{f729}";
pub const KEY_END: &str = "\u{f72b}";
pub const KEY_PAGE_UP: &str = "\u{f72c}";
pub const KEY_PAGE_DOWN: &str = "\u{f72d}";

/// Where a list selection lands after Home, End, PageUp, or PageDown.
///
/// `Home` selects the first row, `End` the last, `PageUp`/`PageDown` step by `page` rows the
/// way an arrow key steps by one, through [`move_selection`]. Any other key answers `None`, so
/// the `.slint` scope that calls this can tell "not a jump key" apart from "jump landed on
/// row -1", which `Shell.jump` cannot: it folds `None` into `-1` too, so the scope tests the
/// key against `Key.Home` and friends before it ever calls `Shell.jump`.
pub fn jump(key: &str, current: i32, page: i32, len: i32) -> Option<i32> {
    match key {
        KEY_HOME => Some(if len <= 0 { -1 } else { 0 }),
        KEY_END => Some(if len <= 0 { -1 } else { len - 1 }),
        KEY_PAGE_UP => Some(move_selection(current, -page, len)),
        KEY_PAGE_DOWN => Some(move_selection(current, page, len)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KEY_END, KEY_HOME, KEY_PAGE_DOWN, KEY_PAGE_UP, jump, key_to_screen, move_selection,
    };
    use crate::Screen;

    #[test]
    fn the_five_digits_name_the_five_screens() {
        assert_eq!(key_to_screen("1"), Some(Screen::Instances));
        assert_eq!(key_to_screen("2"), Some(Screen::Instance));
        assert_eq!(key_to_screen("3"), Some(Screen::Browser));
        assert_eq!(key_to_screen("4"), Some(Screen::Accounts));
        assert_eq!(key_to_screen("5"), Some(Screen::Settings));
    }

    #[test]
    fn any_other_key_names_no_screen() {
        assert_eq!(key_to_screen("0"), None);
        assert_eq!(key_to_screen("6"), None);
        assert_eq!(key_to_screen("a"), None);
        assert_eq!(key_to_screen(""), None);
    }

    #[test]
    fn selection_steps_and_stops_at_both_ends() {
        assert_eq!(move_selection(0, 1, 3), 1);
        assert_eq!(move_selection(2, 1, 3), 2, "the last row is the end");
        assert_eq!(move_selection(1, -1, 3), 0);
        assert_eq!(move_selection(0, -1, 3), 0, "the first row is the end");
    }

    #[test]
    fn an_unchosen_list_starts_at_the_near_end() {
        assert_eq!(move_selection(-1, 1, 3), 0);
        assert_eq!(move_selection(-1, -1, 3), 2);
    }

    #[test]
    fn an_empty_list_has_no_selection() {
        assert_eq!(move_selection(-1, 1, 0), -1);
        assert_eq!(move_selection(2, -1, 0), -1);
    }

    #[test]
    fn a_stale_index_is_pulled_back_into_the_list() {
        assert_eq!(move_selection(9, 1, 3), 2);
        assert_eq!(move_selection(i32::MAX, 1, 3), 2, "no wrap on overflow");
    }

    #[test]
    fn jump_home_selects_the_first_row() {
        assert_eq!(jump(KEY_HOME, 2, 5, 10), Some(0));
        assert_eq!(jump(KEY_HOME, -1, 5, 10), Some(0));
    }

    #[test]
    fn jump_end_selects_the_last_row() {
        assert_eq!(jump(KEY_END, 0, 5, 10), Some(9));
        assert_eq!(jump(KEY_END, -1, 5, 10), Some(9));
    }

    #[test]
    fn jump_page_up_steps_back_a_page_and_stops_at_the_first_row() {
        assert_eq!(jump(KEY_PAGE_UP, 7, 5, 10), Some(2));
        assert_eq!(
            jump(KEY_PAGE_UP, 1, 5, 10),
            Some(0),
            "the first row is the end"
        );
    }

    #[test]
    fn jump_page_down_steps_forward_a_page_and_stops_at_the_last_row() {
        assert_eq!(jump(KEY_PAGE_DOWN, 2, 5, 10), Some(7));
        assert_eq!(
            jump(KEY_PAGE_DOWN, 8, 5, 10),
            Some(9),
            "the last row is the end"
        );
    }

    #[test]
    fn jump_on_an_empty_list_answers_no_selection() {
        assert_eq!(jump(KEY_HOME, -1, 5, 0), Some(-1));
        assert_eq!(jump(KEY_END, -1, 5, 0), Some(-1));
        assert_eq!(jump(KEY_PAGE_UP, -1, 5, 0), Some(-1));
        assert_eq!(jump(KEY_PAGE_DOWN, -1, 5, 0), Some(-1));
    }

    #[test]
    fn a_non_jump_key_names_no_jump() {
        assert_eq!(jump("a", 0, 5, 10), None);
        // Up arrow, another private-use code point: not one of the four jump keys.
        assert_eq!(jump("\u{f700}", 0, 5, 10), None);
        assert_eq!(jump("", 0, 5, 10), None);
    }
}
