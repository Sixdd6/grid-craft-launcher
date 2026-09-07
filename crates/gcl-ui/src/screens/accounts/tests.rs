//! Unit tests for the accounts screen's pure helpers. No Slint instance and no display needed.

use gcl_core::auth::offline::offline_account;
use gcl_core::auth::{Account, AccountKind};

use super::{account_status, added_status, is_cancelled, remove_text, signed_in_status};

#[test]
fn account_status_names_the_empty_list() {
    assert_eq!(account_status(0), "No accounts yet");
    assert_eq!(account_status(1), "1 account(s)");
    assert_eq!(account_status(3), "3 account(s)");
}

#[test]
fn added_status_says_which_kind_was_added() {
    let offline = offline_account("Notch");
    assert_eq!(added_status(&offline), "added offline account Notch");

    let msa = Account {
        kind: AccountKind::Msa,
        ..offline_account("Steve")
    };
    assert_eq!(added_status(&msa), "added Microsoft account Steve");
}

#[test]
fn signed_in_status_names_the_account() {
    assert_eq!(signed_in_status("Steve"), "signed in as Steve");
}

#[test]
fn remove_text_says_the_account_is_only_dropped_here() {
    let text = remove_text("Steve");
    assert!(text.starts_with("Steve is removed"), "got {text}");
    assert!(
        text.contains("Nothing is changed at Microsoft"),
        "got {text}"
    );
}

#[test]
fn is_cancelled_only_matches_a_cancelled_sign_in() {
    let cancelled = gcl_core::Error::Auth(gcl_core::auth::Error::Cancelled);
    assert!(is_cancelled(&cancelled));

    let expired = gcl_core::Error::Auth(gcl_core::auth::Error::DeviceCodeExpired);
    assert!(!is_cancelled(&expired), "an expired code is a real failure");

    let io = gcl_core::Error::Io(std::io::Error::other("disk on fire"));
    assert!(!is_cancelled(&io));
}
