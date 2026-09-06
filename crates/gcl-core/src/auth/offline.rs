//! Offline accounts: a deterministic UUID derived from the player name, matching Java's
//! `UUID.nameUUIDFromBytes`.

use md5::{Digest, Md5};

use super::{Account, AccountKind};

/// Computes the offline-player UUID for `name`, matching Java's
/// `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`.
///
/// This is not `uuid::Uuid::new_v3` with the nil namespace: Java hashes the bytes directly
/// with no namespace prefix, then sets the version and variant bits on the raw MD5 digest.
pub fn offline_uuid(name: &str) -> uuid::Uuid {
    let mut hasher = Md5::new();
    hasher.update(b"OfflinePlayer:");
    hasher.update(name.as_bytes());
    let mut bytes: [u8; 16] = hasher.finalize().into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

/// Builds an offline [`Account`] for `name`, with no token, expiry, or xuid.
pub fn offline_account(name: &str) -> Account {
    Account {
        id: offline_uuid(name).to_string(),
        name: name.to_string(),
        kind: AccountKind::Offline,
        mc_token: None,
        mc_token_expires: None,
        xuid: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notch_matches_the_known_java_value() {
        let expected: uuid::Uuid = "b50ad385-829d-3141-a216-7e7d7539ba7f".parse().unwrap();
        assert_eq!(offline_uuid("Notch"), expected);
    }

    #[test]
    fn the_uuid_is_case_sensitive_to_the_name() {
        assert_ne!(offline_uuid("Notch"), offline_uuid("notch"));
    }

    #[test]
    fn offline_account_uses_the_dashed_lowercase_uuid_as_the_id() {
        let account = offline_account("Notch");
        assert_eq!(account.id, "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(account.name, "Notch");
        assert_eq!(account.kind, AccountKind::Offline);
    }
}
