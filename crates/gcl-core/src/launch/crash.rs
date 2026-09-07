//! Turns a game log into the one-line crash hint a non-zero exit is reported with (SPEC R11.5).

use std::path::Path;

/// Log markers that usually carry the reason a Minecraft launch died, most telling first.
///
/// A stack trace holds both a `Caused by` line and the generic exception line that wraps it, so
/// the more specific marker is looked for across the whole tail before the next one is tried.
const MARKERS: [&str; 4] = ["Caused by", "Mod File:", "Mixin", "Exception"];

/// Markers that only name a Java word, so an INFO line that happens to hold one is skipped.
///
/// `Mixin` names the mixin subsystem, which logs at INFO all through boot, and `Exception` shows
/// up in ordinary INFO chatter. `Caused by` and `Mod File:` are written by a real failure, so a
/// line holding one counts whatever its level is.
const LEVEL_CHECKED: [&str; 2] = ["Mixin", "Exception"];

/// How many lines from the end of the log are scanned.
const TAIL_LINES: usize = 200;

/// Longest hint text, in characters. Longer lines are cut and suffixed with `…`.
const MAX_CHARS: usize = 160;

/// What a hint says when the log holds no marker line.
pub const GENERIC_HINT: &str = "non-zero exit; read the log";

/// Reads `log` and returns a one-line reason for the exit.
///
/// It scans the last 200 lines for a line naming a cause, a mod file, a mixin, or an exception,
/// in that order, and quotes the last one it finds trimmed to 160 characters. An unreadable
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
///
/// Two rules pick the line:
///
/// * Recency. Each marker is looked for from the end of the tail backwards, so the newest line
///   holding it wins. A boot-time failure — the `401` on `/player/attributes` an offline account
///   always gets — cannot mask a crash that came after it.
/// * Level. For [`LEVEL_CHECKED`] markers an INFO line is skipped, read off the plain format
///   `[HH:MM:SS] [thread/LEVEL]: …` that [`crate::launch::log4j`] writes.
///
/// A run whose only marker is that boot-time `401` still gets it as the hint: it is the one
/// failure the log names, and the level rule leaves it alone because it is logged at ERROR.
pub fn hint_from_log(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(TAIL_LINES)..];
    for marker in MARKERS {
        let checked = LEVEL_CHECKED.contains(&marker);
        let found = tail
            .iter()
            .rev()
            .map(|line| line.trim())
            .find(|line| line.contains(marker) && !(checked && is_info(line)));
        if let Some(line) = found {
            return trim_to(line, MAX_CHARS);
        }
    }
    GENERIC_HINT.to_string()
}

/// Whether a converted line reports at INFO.
///
/// The plain format is `[HH:MM:SS] [thread/LEVEL]: message`, so the level is the text after the
/// last `/` of the second bracket group. A line in any other shape has no level and is kept.
fn is_info(line: &str) -> bool {
    plain_level(line) == Some("INFO")
}

/// The level named by a converted line's `[thread/LEVEL]` group, if it has one.
fn plain_level(line: &str) -> Option<&str> {
    let after_time = line.strip_prefix('[')?.split_once("] [")?.1;
    let source = after_time.split_once("]:")?.0;
    Some(source.rsplit('/').next().unwrap_or(source))
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
    fn the_most_recent_marker_line_in_the_tail_is_quoted() {
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

    /// What `launch::log4j` makes of the boot-time authlib event every offline account gets.
    /// The same lines are in `tests/fixtures/launch/authlib_401_event.log` as raw XML.
    const OFFLINE_401: &str = "[16:58:59] [Download-2/ERROR]: Failed to fetch user properties\n\
         com.mojang.authlib.exceptions.InvalidCredentialsException: Status: 401\n\
         \tat knot//com.mojang.authlib.yggdrasil.YggdrasilUserApiService.fetchProperties(YggdrasilUserApiService.java:173)\n\
         Caused by: MinecraftClientHttpException[type=HTTP_ERROR, status=401, response=ErrorResponse[path=/player/attributes, error=null]]\n\
         \t... 5 more\n";

    #[test]
    fn a_crash_after_the_boot_time_401_is_the_hint() {
        let log = format!(
            "{OFFLINE_401}\
             [17:01:12] [Render thread/INFO]: Loaded 3 mods\n\
             [17:01:13] [Render thread/ERROR]: Uncaught exception in thread \"Render thread\"\n\
             Caused by: java.lang.NoSuchMethodError: net.minecraft.class_310.method_1551()\n\
             \tat Main.main(Main.java:1)\n"
        );
        assert_eq!(
            hint_from_log(&log),
            "Caused by: java.lang.NoSuchMethodError: net.minecraft.class_310.method_1551()",
            "the newest cause wins over the one from boot"
        );
    }

    #[test]
    fn an_offline_run_with_only_the_401_reports_that_401() {
        // It is the one failure the log names, so it is what the hint says.
        let hint = hint_from_log(OFFLINE_401);
        assert!(
            hint.starts_with("Caused by: MinecraftClientHttpException"),
            "{hint}"
        );
        assert!(hint.contains("/player/attributes"), "{hint}");
    }

    #[test]
    fn an_info_line_is_skipped_for_a_mixin_or_exception_marker() {
        let log = "[16:58:55] [main/INFO]: Compatibility level set to JAVA_21 by Mixin\n\
                   [16:58:56] [main/INFO]: caught an Exception and carried on\n\
                   [16:58:57] [main/INFO]: done\n";
        assert_eq!(hint_from_log(log), GENERIC_HINT);
    }

    #[test]
    fn an_error_exception_line_beats_a_later_info_one() {
        let log = "[16:58:55] [main/ERROR]: java.lang.IllegalStateException: mixin apply failed\n\
                   [16:58:56] [main/INFO]: recovered, Exception logged above\n";
        assert_eq!(
            hint_from_log(log),
            "[16:58:55] [main/ERROR]: java.lang.IllegalStateException: mixin apply failed"
        );
    }
}
