//! Sha1 helpers for the content-addressed download cache.

use std::io::Read;
use std::path::Path;

use sha1::{Digest, Sha1};

/// Reads a file in chunks and returns its sha1 as lowercase hex.
///
/// Blocking: call it inside `tokio::task::spawn_blocking`.
pub fn sha1_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Returns the sha1 of bytes already in memory, as lowercase hex.
pub fn sha1_hex(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sha1_hex_matches_known_vector() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn sha1_hex_of_empty_input() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn sha1_file_matches_sha1_hex_for_large_input() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("blob.bin");
        let payload = vec![3u8; 200_000];
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(&payload).expect("write");
        drop(f);
        assert_eq!(sha1_file(&path).expect("hash file"), sha1_hex(&payload));
    }

    #[test]
    fn sha1_file_reports_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(sha1_file(&dir.path().join("nope")).is_err());
    }
}
