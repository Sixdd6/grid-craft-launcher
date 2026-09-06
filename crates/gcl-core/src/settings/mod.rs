//! `options.txt` preseed and keyed overrides.
//!
//! Minecraft's `options.txt` holds one `key:value` per line. See the `instance-model` skill,
//! "options.txt semantics". This module does not depend on `instances`: callers pass a game
//! directory and a map, so `instances::create` and `launcher` can both use it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::paths::write_atomic;

/// File name of the settings file inside a game directory.
const OPTIONS_FILE: &str = "options.txt";

/// Errors reading or writing `options.txt`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O operation on `options.txt` failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// One line of `options.txt`: a `key:value` pair, or any other line kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// A `key:value` line, split at the first `:`.
    Pair {
        /// The key, the text before the first `:`.
        key: String,
        /// The value, the text after the first `:`, kept untouched (no trimming).
        value: String,
    },
    /// A line with no `:`, or any line the parser did not turn into a pair.
    Other(String),
}

/// The parsed contents of an `options.txt` file, preserving line order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptionsFile {
    lines: Vec<Line>,
    /// Whether the source text used `\r\n` line endings. Set from the parsed input and
    /// carried through to [`OptionsFile::to_string`] so a Windows-style `options.txt` round
    /// trips byte-for-byte.
    crlf: bool,
}

impl OptionsFile {
    /// Parses `text` into lines, splitting each at its first `:` into a key/value pair.
    ///
    /// A line with no `:` is kept as-is. Trailing newline handling is symmetric with
    /// [`OptionsFile::to_string`]: an empty or all-blank input yields no lines. If any line in
    /// `text` ends with `\r\n`, [`OptionsFile::to_string`] emits `\r\n` for every line.
    pub fn parse(text: &str) -> OptionsFile {
        let mut lines = Vec::new();
        for raw in text.lines() {
            match raw.split_once(':') {
                Some((key, value)) => lines.push(Line::Pair {
                    key: key.to_string(),
                    value: value.to_string(),
                }),
                None => lines.push(Line::Other(raw.to_string())),
            }
        }
        OptionsFile {
            lines,
            crlf: text.contains("\r\n"),
        }
    }

    /// Returns the value for `key`, if a `Pair` line with that key exists.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().find_map(|line| match line {
            Line::Pair { key: k, value } if k == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// Every `key:value` pair, in file order. Lines with no `:` are skipped.
    pub fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.lines.iter().filter_map(|line| match line {
            Line::Pair { key, value } => Some((key.as_str(), value.as_str())),
            Line::Other(_) => None,
        })
    }

    /// Sets `key` to `value`, replacing the first matching `Pair` line in place, or appending
    /// a new line when no line has that key.
    ///
    /// Returns `true` when the file actually changed: a new line was appended, or an existing
    /// line's value differed from `value`. Returns `false` when the key already held exactly
    /// `value`, so a caller can skip rewriting an unchanged file.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        for line in &mut self.lines {
            if let Line::Pair { key: k, value: v } = line
                && k == key
            {
                if v == value {
                    return false;
                }
                *v = value.to_string();
                return true;
            }
        }
        self.lines.push(Line::Pair {
            key: key.to_string(),
            value: value.to_string(),
        });
        true
    }

    /// Removes the first `Pair` line with the given key. Returns whether one was removed.
    pub fn remove(&mut self, key: &str) -> bool {
        let pos = self.lines.iter().position(|line| match line {
            Line::Pair { key: k, .. } => k == key,
            Line::Other(_) => false,
        });
        match pos {
            Some(i) => {
                self.lines.remove(i);
                true
            }
            None => false,
        }
    }

    /// Renders back to `options.txt` text.
    ///
    /// Ends with exactly one trailing newline when non-empty; is the empty string when there
    /// are no lines.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let newline = if self.crlf { "\r\n" } else { "\n" };
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Pair { key, value } => {
                    out.push_str(key);
                    out.push(':');
                    out.push_str(value);
                }
                Line::Other(text) => out.push_str(text),
            }
            out.push_str(newline);
        }
        out
    }
}

/// Reads and parses `options.txt` at `path`. A missing file yields an empty [`OptionsFile`].
pub fn read(path: &Path) -> Result<OptionsFile, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(OptionsFile::parse(&text)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(OptionsFile::default()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Writes `f` to `path` through a temp file and a rename.
pub fn write(path: &Path, f: &OptionsFile) -> Result<(), Error> {
    write_atomic(path, f.to_string().as_bytes()).map_err(|err| match err {
        crate::paths::Error::Io { path, source } => Error::Io { path, source },
        other => Error::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other(other.to_string()),
        },
    })
}

/// Applies `overrides` to `options.txt` in `game_dir`: for each key, replaces the line
/// starting with `key:`, or appends `key:value` if absent. Every other line is left
/// untouched, in its original order.
///
/// Returns the number of keys whose value actually changed (added or different from what was
/// already there). The file is only rewritten when that count is non-zero, so applying the
/// same overrides twice leaves the file untouched on the second call.
pub fn apply_overrides_to(
    game_dir: &Path,
    overrides: &BTreeMap<String, String>,
) -> Result<usize, Error> {
    let path = game_dir.join(OPTIONS_FILE);
    let mut file = read(&path)?;
    let mut changed = 0usize;
    for (key, value) in overrides {
        if file.set(key, value) {
            changed += 1;
        }
    }
    if changed > 0 {
        write(&path, &file)?;
    }
    Ok(changed)
}

/// Preseeds `options.txt` in `game_dir` from `defaults`, but only when the file does not
/// already exist. An existing file, even an empty one, is left untouched.
pub fn apply_preseed(game_dir: &Path, defaults: &BTreeMap<String, String>) -> Result<(), Error> {
    let path = game_dir.join(OPTIONS_FILE);
    if path.exists() || defaults.is_empty() {
        return Ok(());
    }
    let mut file = OptionsFile::default();
    for (key, value) in defaults {
        file.set(key, value);
    }
    write(&path, &file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn parse_and_to_string_round_trips_order_and_other_lines() {
        let text = "renderDistance:8\n# a comment\nguiScale:2\nnoColonHere\n";
        let file = OptionsFile::parse(text);
        assert_eq!(file.to_string(), text);
    }

    #[test]
    fn pairs_lists_key_value_lines_in_order_and_skips_the_rest() {
        let file = OptionsFile::parse("renderDistance:8\n# a comment\nguiScale:2\n");
        let pairs: Vec<(&str, &str)> = file.pairs().collect();
        assert_eq!(pairs, vec![("renderDistance", "8"), ("guiScale", "2")]);
    }

    #[test]
    fn parse_splits_at_first_colon_only() {
        let file = OptionsFile::parse("resourcePacks:[\"a:b\"]\n");
        assert_eq!(file.get("resourcePacks"), Some("[\"a:b\"]"));
    }

    #[test]
    fn set_writes_a_line_with_a_colon_inside_the_value() {
        let mut file = OptionsFile::default();
        file.set("resourcePacks", "[\"a:b\"]");
        assert_eq!(file.to_string(), "resourcePacks:[\"a:b\"]\n");
    }

    #[test]
    fn empty_text_round_trips_to_empty_string() {
        let file = OptionsFile::parse("");
        assert_eq!(file.to_string(), "");
    }

    #[test]
    fn crlf_input_round_trips_byte_identical() {
        let text = "renderDistance:8\r\nguiScale:2\r\n";
        let file = OptionsFile::parse(text);
        assert_eq!(file.to_string(), text);
    }

    #[test]
    fn set_on_a_crlf_file_keeps_crlf_line_endings() {
        let mut file = OptionsFile::parse("renderDistance:8\r\nguiScale:2\r\n");
        file.set("renderDistance", "16");
        file.set("lang", "en_us");
        assert_eq!(
            file.to_string(),
            "renderDistance:16\r\nguiScale:2\r\nlang:en_us\r\n"
        );
    }

    #[test]
    fn set_replaces_an_existing_key_in_place() {
        let mut file = OptionsFile::parse("a:1\nb:2\n");
        assert!(file.set("a", "9"));
        assert_eq!(file.to_string(), "a:9\nb:2\n");
    }

    #[test]
    fn set_returns_false_when_the_value_is_unchanged() {
        let mut file = OptionsFile::parse("a:1\n");
        assert!(!file.set("a", "1"));
        assert_eq!(file.to_string(), "a:1\n");
    }

    #[test]
    fn set_with_a_duplicate_key_replaces_only_the_first_occurrence() {
        let mut file = OptionsFile::parse("a:1\nb:2\na:3\n");
        assert!(file.set("a", "9"));
        assert_eq!(file.get("a"), Some("9"));
        assert_eq!(file.to_string(), "a:9\nb:2\na:3\n");
    }

    #[test]
    fn set_appends_a_new_key() {
        let mut file = OptionsFile::parse("a:1\n");
        file.set("b", "2");
        assert_eq!(file.to_string(), "a:1\nb:2\n");
    }

    #[test]
    fn get_returns_none_for_a_missing_key() {
        let file = OptionsFile::parse("a:1\n");
        assert_eq!(file.get("z"), None);
    }

    #[test]
    fn remove_deletes_a_matching_line_and_reports_it() {
        let mut file = OptionsFile::parse("a:1\nb:2\n");
        assert!(file.remove("a"));
        assert_eq!(file.to_string(), "b:2\n");
        assert!(!file.remove("a"));
    }

    #[test]
    fn apply_overrides_to_replaces_existing_and_appends_new_keys_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path();
        std::fs::write(
            game_dir.join("options.txt"),
            "renderDistance:8\nguiScale:2\n",
        )
        .unwrap();
        let overrides = defaults(&[("renderDistance", "16"), ("lang", "en_us")]);
        let changed = apply_overrides_to(game_dir, &overrides).unwrap();
        assert_eq!(changed, 2);
        let text = std::fs::read_to_string(game_dir.join("options.txt")).unwrap();
        assert_eq!(text, "renderDistance:16\nguiScale:2\nlang:en_us\n");
    }

    #[test]
    fn apply_overrides_to_is_a_no_op_on_a_second_identical_call() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path();
        std::fs::write(
            game_dir.join("options.txt"),
            "renderDistance:8\nguiScale:2\n",
        )
        .unwrap();
        let overrides = defaults(&[("renderDistance", "16"), ("lang", "en_us")]);

        let first = apply_overrides_to(game_dir, &overrides).unwrap();
        assert_eq!(first, 2);
        let path = game_dir.join("options.txt");
        let after_first = std::fs::read(&path).unwrap();
        let mtime_first = std::fs::metadata(&path).unwrap().modified().unwrap();

        let second = apply_overrides_to(game_dir, &overrides).unwrap();
        assert_eq!(second, 0);
        let after_second = std::fs::read(&path).unwrap();
        let mtime_second = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(after_first, after_second);
        assert_eq!(
            mtime_first, mtime_second,
            "file was rewritten on a no-op call"
        );
    }

    #[test]
    fn apply_overrides_to_creates_the_file_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let overrides = defaults(&[("renderDistance", "16")]);
        apply_overrides_to(dir.path(), &overrides).unwrap();
        let text = std::fs::read_to_string(dir.path().join("options.txt")).unwrap();
        assert_eq!(text, "renderDistance:16\n");
    }

    #[test]
    fn apply_preseed_writes_defaults_when_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        apply_preseed(dir.path(), &defaults(&[("renderDistance", "12")])).unwrap();
        let text = std::fs::read_to_string(dir.path().join("options.txt")).unwrap();
        assert_eq!(text, "renderDistance:12\n");
    }

    #[test]
    fn apply_preseed_does_not_touch_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("options.txt"), "already:here\n").unwrap();
        apply_preseed(dir.path(), &defaults(&[("renderDistance", "12")])).unwrap();
        let text = std::fs::read_to_string(dir.path().join("options.txt")).unwrap();
        assert_eq!(text, "already:here\n");
    }

    #[test]
    fn apply_preseed_writes_nothing_when_defaults_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        apply_preseed(dir.path(), &BTreeMap::new()).unwrap();
        assert!(!dir.path().join("options.txt").exists());
    }

    #[test]
    fn read_of_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = read(&dir.path().join("options.txt")).unwrap();
        assert_eq!(file.to_string(), "");
    }
}
