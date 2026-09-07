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

#[cfg(test)]
mod tests {
    use super::{key_to_screen, move_selection};
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
}
