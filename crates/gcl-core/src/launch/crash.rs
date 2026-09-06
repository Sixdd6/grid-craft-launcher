//! Turns a game log into the one-line crash hint a non-zero exit is reported with (SPEC R11.5).

use std::path::Path;

/// Log markers that usually carry the reason a Minecraft launch died.
const MARKERS: [&str; 4] = ["Exception", "Caused by", "Mod File:", "Mixin"];

/// How many lines from the end of the log are scanned.
const TAIL_LINES: usize = 200;

/// Longest hint text, in characters. Longer lines are cut and suffixed with `…`.
const MAX_CHARS: usize = 160;

/// What a hint says when the log holds no marker line.
pub const GENERIC_HINT: &str = "non-zero exit; read the log";

/// Reads `log` and returns a one-line reason for the exit.
///
/// It scans the last [`TAIL_LINES`] lines for the first line naming an exception, a cause, a
/// mod file, or a mixin, and quotes it trimmed to [`MAX_CHARS`] characters. An unreadable log,
/// or one with no such line, gives [`GENERIC_HINT`].
pub fn crash_hint(log: &Path) -> String {
    match std::fs::read_to_string(log) {
        Ok(text) => hint_from_log(&text),
        Err(source) => {
            tracing::warn!(path = %log.display(), %source, "could not read the game log for a hint");
            GENERIC_HINT.to_string()
        }
    }
}

/// The hint for already-read log text. Split out so a test can plant a log in memory.
pub fn hint_from_log(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(TAIL_LINES)..];
    match tail
        .iter()
        .map(|line| line.trim())
        .find(|line| MARKERS.iter().any(|marker| line.contains(marker)))
    {
        Some(line) => trim_to(line, MAX_CHARS),
        None => GENERIC_HINT.to_string(),
    }
}

/// Cuts `line` to `max` characters, marking a cut with a trailing `…`.
fn trim_to(line: &str, max: usize) -> String {
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_marker_line_in_the_tail_is_quoted() {
        let log = "[main] INFO starting\n\
                   [main] ERROR Caused by: java.lang.NoSuchMethodError: sodium\n\
                   [main] ERROR java.lang.Exception: later\n";
        assert_eq!(
            hint_from_log(log),
            "[main] ERROR Caused by: java.lang.NoSuchMethodError: sodium"
        );
    }

    #[test]
    fn a_mod_file_line_counts_as_a_marker_and_is_trimmed() {
        let line = format!("Mod File: {}", "a".repeat(400));
        let hint = hint_from_log(&format!("noise\n{line}\n"));
        assert_eq!(hint.chars().count(), MAX_CHARS + 1, "{hint}");
        assert!(hint.starts_with("Mod File: aaa"), "{hint}");
        assert!(hint.ends_with('…'), "{hint}");
    }

    #[test]
    fn only_the_last_two_hundred_lines_are_scanned() {
        let mut log = String::from("Mixin apply failed very early\n");
        for i in 0..TAIL_LINES {
            log.push_str(&format!("line {i}\n"));
        }
        assert_eq!(hint_from_log(&log), GENERIC_HINT);
    }

    #[test]
    fn a_log_without_a_marker_gets_the_generic_hint() {
        assert_eq!(hint_from_log("clean run\nbye\n"), GENERIC_HINT);
    }

    #[test]
    fn a_planted_log_file_is_read_from_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("game.log");
        std::fs::write(&path, "boot\n\tMixin transformation failed for sodium\n").expect("write");
        assert_eq!(
            crash_hint(&path),
            "Mixin transformation failed for sodium",
            "the line is trimmed of leading whitespace"
        );
    }

    #[test]
    fn a_missing_log_gets_the_generic_hint() {
        assert_eq!(crash_hint(Path::new("/nonexistent/game.log")), GENERIC_HINT);
    }
}
