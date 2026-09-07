//! Turns a game log into the one-line crash hint a non-zero exit is reported with (SPEC R11.5).

use std::path::Path;

/// Log markers that usually carry the reason a Minecraft launch died, most telling first.
///
/// A stack trace holds both a `Caused by` line and the generic exception line that wraps it, so
/// the more specific marker is looked for across the whole tail before the next one is tried.
const MARKERS: [&str; 4] = ["Caused by", "Mod File:", "Mixin", "Exception"];

/// How many lines from the end of the log are scanned.
const TAIL_LINES: usize = 200;

/// Longest hint text, in characters. Longer lines are cut and suffixed with `…`.
const MAX_CHARS: usize = 160;

/// What a hint says when the log holds no marker line.
pub const GENERIC_HINT: &str = "non-zero exit; read the log";

/// Reads `log` and returns a one-line reason for the exit.
///
/// It scans the last 200 lines for a line naming a cause, a mod file, a mixin, or an exception,
/// in that order, and quotes the first one it finds trimmed to 160 characters. An unreadable
/// log, or one with no such line, gives [`GENERIC_HINT`].
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
    for marker in MARKERS {
        if let Some(line) = tail
            .iter()
            .map(|line| line.trim())
            .find(|line| line.contains(marker))
        {
            return trim_to(line, MAX_CHARS);
        }
    }
    GENERIC_HINT.to_string()
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
    fn a_cause_beats_a_generic_exception_that_came_first() {
        let log = "java.lang.RuntimeException: something went wrong\n\
                   \tat Main.main(Main.java:1)\n\
                   Caused by: java.lang.NoSuchMethodError: sodium\n";
        assert_eq!(
            hint_from_log(log),
            "Caused by: java.lang.NoSuchMethodError: sodium"
        );
    }

    #[test]
    fn a_mod_file_beats_a_generic_exception_that_came_first() {
        let log = "java.lang.RuntimeException: something went wrong\n\
                   Mod File: /mods/sodium-fabric-0.5.8.jar\n";
        assert_eq!(
            hint_from_log(log),
            "Mod File: /mods/sodium-fabric-0.5.8.jar"
        );
    }

    #[test]
    fn a_converted_realms_failure_reads_as_plain_text() {
        // What `launch::log4j` makes of the RealmsAvailability event in a real 26.2 game log.
        let log = "[16:59:01] [Download-2/INFO]: Could not authorize you against Realms server\n\
                   [16:59:01] [Download-2/ERROR]: Couldn't connect to realms\n\
                   com.mojang.realmsclient.exception.RealmsServiceException: Realms authentication error\n\
                   \tat knot//com.mojang.realmsclient.client.RealmsClient.execute(RealmsClient.java:528)\n";
        let hint = hint_from_log(log);
        assert_eq!(
            hint,
            "com.mojang.realmsclient.exception.RealmsServiceException: Realms authentication error"
        );
        assert!(!hint.contains("log4j"), "{hint}");
        assert!(!hint.contains("CDATA"), "{hint}");
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
