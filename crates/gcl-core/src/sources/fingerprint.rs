//! CurseForge file fingerprints.
//!
//! A fingerprint is MurmurHash2 (32-bit, the original `MurmurHash2`, not
//! `MurmurHash2A`) with seed `1`, taken over the file bytes after every byte equal to
//! 9, 10, 13, or 32 is removed. CurseForge uses it to identify a jar the launcher
//! already has on disk, including one the user downloaded by hand.

use std::path::Path;

/// Seed CurseForge hashes with.
const SEED: u32 = 1;

/// Bytes CurseForge strips before hashing: tab, line feed, carriage return, space.
const STRIPPED: [u8; 4] = [9, 10, 13, 32];

/// Computes the CurseForge fingerprint of a file's bytes.
pub fn curseforge_fingerprint(bytes: &[u8]) -> u32 {
    let filtered: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|b| !STRIPPED.contains(b))
        .collect();
    murmur2::murmur2(&filtered, SEED)
}

/// Reads a file and computes its CurseForge fingerprint.
///
/// This blocks: it reads the whole file into memory. Async callers wrap it in
/// [`tokio::task::spawn_blocking`].
pub fn fingerprint_file(path: &Path) -> std::io::Result<u32> {
    let bytes = std::fs::read(path)?;
    Ok(curseforge_fingerprint(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed input every test below hashes. It carries all four stripped bytes.
    const INPUT: &[u8] = b"GRID Craft\tLauncher\r\n fingerprint";

    #[test]
    fn fingerprint_matches_a_pinned_vector() {
        // Pinned so a change of crate, seed, or strip set fails loudly.
        assert_eq!(curseforge_fingerprint(INPUT), PINNED);
    }

    /// Pinned value for [`INPUT`], recorded from `murmur2::murmur2` with seed 1 over
    /// the whitespace-stripped bytes.
    const PINNED: u32 = 1_942_052_554;

    #[test]
    fn stripping_whitespace_changes_the_hash() {
        let raw = murmur2::murmur2(INPUT, 1);
        let stripped: Vec<u8> = INPUT
            .iter()
            .copied()
            .filter(|b| !STRIPPED.contains(b))
            .collect();
        assert_eq!(
            curseforge_fingerprint(INPUT),
            murmur2::murmur2(&stripped, 1)
        );
        assert_ne!(
            curseforge_fingerprint(INPUT),
            raw,
            "stripping 9/10/13/32 must change the hash"
        );
    }

    #[test]
    fn fingerprint_file_matches_the_byte_form() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sample.jar");
        std::fs::write(&path, INPUT).expect("write");
        assert_eq!(
            fingerprint_file(&path).expect("read"),
            curseforge_fingerprint(INPUT)
        );
    }

    #[test]
    fn empty_input_hashes_without_panicking() {
        assert_eq!(curseforge_fingerprint(b""), murmur2::murmur2(b"", 1));
    }
}
