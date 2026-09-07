//! Turns Minecraft's log4j XML output into the plain lines the game log, the Logs tab, and
//! [`crate::launch::crash_hint`] read.
//!
//! Mojang starts the game with `-Dlog4j.configurationFile`, so stdout carries one XML element
//! per log event, spread over several lines:
//!
//! ```text
//!   <log4j:Event logger="FabricLoader" timestamp="1788800335660" level="INFO" thread="main">
//!     <log4j:Message><![CDATA[Loading 4 mods:
//!     - fabricloader 0.19.5]]></log4j:Message>
//!     <log4j:Throwable><![CDATA[java.lang.Exception: boom
//!     at Main.main(Main.java:1)]]></log4j:Throwable>
//!   </log4j:Event>
//! ```
//!
//! [`EventParser`] is fed one line at a time and gives back the plain lines a vanilla launcher
//! shows: `[HH:MM:SS] [thread/LEVEL]: message`, then the rest of a multi-line message and every
//! throwable line as records of their own. A line outside an event — stderr, a native library's
//! own output, or an older version that writes plain text — passes through unchanged.
//!
//! One physical line can hold several elements, so the parser walks the remainder after each
//! `]]>` and after each tag until the line is used up. A whole event on one line, a throwable
//! that starts where the message ends, and the `]]]]><![CDATA[>` escape log4j writes for a
//! literal `]]>` are all read the same way.

use std::sync::OnceLock;

use time::format_description::FormatItem;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

use crate::events::LogLevel;

/// The clock part of a converted line, the same shape a vanilla launcher prints.
const TIME_FORMAT: &[FormatItem<'static>] = format_description!("[hour]:[minute]:[second]");

const EVENT_OPEN: &str = "<log4j:Event";
const EVENT_CLOSE: &str = "</log4j:Event>";
const MESSAGE_OPEN: &str = "<log4j:Message>";
const MESSAGE_CLOSE: &str = "</log4j:Message>";
const THROWABLE_OPEN: &str = "<log4j:Throwable>";
const THROWABLE_CLOSE: &str = "</log4j:Throwable>";
const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";
/// Any log4j element, opening or closing. A line inside an event that starts with neither is
/// not XML at all and is passed through.
const ELEMENT_PREFIXES: [&str; 2] = ["<log4j:", "</log4j:"];

/// How many lines one event may buffer before the rest of it is dropped.
const MAX_EVENT_LINES: usize = 20_000;

/// How many bytes of text one event may buffer before the rest of it is dropped.
const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// The line appended in place of everything cut by [`MAX_EVENT_LINES`] or [`MAX_EVENT_BYTES`].
const TRUNCATION_MARK: &str = "… (truncated)";

/// The machine's UTC offset, read once at start-up by [`init_local_offset`].
///
/// `None` means the offset could not be read, and timestamps are then shown in UTC with a
/// trailing `Z` so no one reads them as local time.
static LOCAL_OFFSET: OnceLock<Option<UtcOffset>> = OnceLock::new();

/// Reads the machine's UTC offset and remembers it for every parser built later.
///
/// Call this first thing in `main`, before any thread or async runtime starts: the `time` crate
/// refuses to read the local offset from a process that already has threads, and both binaries
/// start some. When the offset stays unknown, a timestamp is shown in UTC and marked with `Z`.
pub fn init_local_offset() {
    let _ = LOCAL_OFFSET.set(UtcOffset::current_local_offset().ok());
}

/// The offset [`init_local_offset`] stored, or a late read of it, or `None` when unknown.
fn local_offset() -> Option<UtcOffset> {
    *LOCAL_OFFSET.get_or_init(|| UtcOffset::current_local_offset().ok())
}

/// One plain log line, ready for the log file and the event sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    /// The level the line is reported at.
    pub level: LogLevel,
    /// The line itself, with no trailing newline.
    pub text: String,
}

/// Which part of an event the parser is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Between elements: the next text opens a message, a throwable, or closes the event.
    Between,
    /// Inside `<log4j:Message>`'s CDATA.
    Message,
    /// Inside `<log4j:Throwable>`'s CDATA.
    Throwable,
}

/// The event being read, with everything seen of it so far.
#[derive(Debug)]
struct Pending {
    level: LogLevel,
    level_text: String,
    thread: String,
    time: Option<String>,
    message: Vec<String>,
    throwable: Vec<String>,
    section: Section,
    /// Buffered lines so far, counted against [`MAX_EVENT_LINES`].
    lines: usize,
    /// Buffered bytes so far, counted against [`MAX_EVENT_BYTES`].
    bytes: usize,
    /// Whether a cap was hit and buffering stopped.
    truncated: bool,
    /// Whether the first line of this event was already given a `[time] [thread/LEVEL]:` prefix.
    emitted: bool,
}

impl Pending {
    /// Reads the attributes off an opening `<log4j:Event …>` tag.
    fn from_header(header: &str, zone: Option<UtcOffset>) -> Self {
        let level_text = attr(header, "level").unwrap_or_else(|| "INFO".to_string());
        let level_text = level_text.to_uppercase();
        Self {
            level: level_of(&level_text),
            level_text,
            thread: attr(header, "thread").unwrap_or_default(),
            time: attr(header, "timestamp")
                .and_then(|ms| ms.parse::<i64>().ok())
                .and_then(|ms| format_time(ms, zone)),
            message: Vec::new(),
            throwable: Vec::new(),
            section: Section::Between,
            lines: 0,
            bytes: 0,
            truncated: false,
            emitted: false,
        }
    }

    /// Adds CDATA text to the section's buffer and returns what follows the closing `]]>`.
    ///
    /// `None` means the body runs on to the next line. `]]]]><![CDATA[>` is log4j's escape for a
    /// literal `]]>`, so text after `]]>` that opens CDATA again continues the same body on the
    /// same line.
    fn absorb<'a>(&mut self, chunk: &'a str) -> Option<&'a str> {
        let mut rest = chunk;
        let mut same_line = false;
        loop {
            let Some(end) = rest.find(CDATA_CLOSE) else {
                self.buffer(rest, same_line);
                return None;
            };
            self.buffer(&rest[..end], same_line);
            let after = &rest[end + CDATA_CLOSE.len()..];
            match after.strip_prefix(CDATA_OPEN) {
                Some(more) => {
                    rest = more;
                    same_line = true;
                }
                None => return Some(after),
            }
        }
    }

    /// Buffers CDATA text, starting a new line unless it continues the one before it.
    ///
    /// Past either cap the text is dropped and one [`TRUNCATION_MARK`] line takes its place, so
    /// an event that never closes cannot grow without bound.
    fn buffer(&mut self, text: &str, same_line: bool) {
        if self.truncated {
            return;
        }
        self.bytes += text.len();
        if !same_line {
            self.lines += 1;
        }
        if self.lines > MAX_EVENT_LINES || self.bytes > MAX_EVENT_BYTES {
            self.truncated = true;
            self.buf_mut().push(TRUNCATION_MARK.to_string());
            return;
        }
        if same_line && let Some(last) = self.buf_mut().last_mut() {
            last.push_str(text);
            return;
        }
        self.buf_mut().push(text.to_string());
    }

    /// The buffer the current section writes to.
    fn buf_mut(&mut self) -> &mut Vec<String> {
        match self.section {
            Section::Throwable => &mut self.throwable,
            _ => &mut self.message,
        }
    }

    /// The plain lines buffered so far, leaving the event empty.
    ///
    /// The first message line carries the `[HH:MM:SS] [thread/LEVEL]: ` prefix, once per event.
    /// Every later message line and every throwable line is its own record at the same level,
    /// verbatim, so a stack trace keeps its leading tabs. A throwable's CDATA usually ends on a
    /// line of its own, and that trailing empty line is dropped.
    fn emit(&mut self) -> Vec<LogRecord> {
        let stamp = match &self.time {
            Some(time) => format!("[{time}] "),
            None => String::new(),
        };
        let source = if self.thread.is_empty() {
            self.level_text.clone()
        } else {
            format!("{}/{}", self.thread, self.level_text)
        };
        let level = self.level;
        let message = std::mem::take(&mut self.message);
        let mut throwable = std::mem::take(&mut self.throwable);
        if throwable.last().is_some_and(|line| line.is_empty()) {
            throwable.pop();
        }
        let mut out = Vec::with_capacity(message.len() + throwable.len());
        let mut message = message.into_iter();
        if !self.emitted {
            self.emitted = true;
            if let Some(first) = message.next() {
                out.push(LogRecord {
                    level,
                    text: format!("{stamp}[{source}]: {first}"),
                });
            }
        }
        for line in message.chain(throwable) {
            out.push(LogRecord { level, text: line });
        }
        out
    }
}

/// A streaming reader that turns log4j XML lines into plain log lines.
///
/// Feed it every line of one stream with [`push`](EventParser::push), in order, and call
/// [`flush`](EventParser::flush) at end of stream so a half-written event is not lost.
#[derive(Debug)]
pub struct EventParser {
    /// The level given to a line that is not part of an event.
    fallback: LogLevel,
    /// The offset event timestamps are shown in, or `None` for UTC marked with `Z`.
    zone: Option<UtcOffset>,
    /// The event being read, if any.
    pending: Option<Pending>,
}

impl EventParser {
    /// A parser that shows timestamps in local time and reports plain lines at `fallback`.
    ///
    /// Local time comes from [`init_local_offset`]. Without it the timestamps read as UTC and
    /// end in `Z`.
    pub fn new(fallback: LogLevel) -> Self {
        Self::with_zone(fallback, local_offset())
    }

    /// The same, with the time zone given rather than the machine's. For tests.
    pub fn with_offset(fallback: LogLevel, offset: UtcOffset) -> Self {
        Self::with_zone(fallback, Some(offset))
    }

    /// The one constructor: `None` means the offset is unknown and timestamps are UTC with `Z`.
    fn with_zone(fallback: LogLevel, zone: Option<UtcOffset>) -> Self {
        Self {
            fallback,
            zone,
            pending: None,
        }
    }

    /// Feeds one line and returns the plain lines it completed, if any.
    pub fn push(&mut self, line: &str) -> Vec<LogRecord> {
        let mut out = Vec::new();
        self.push_into(line, &mut out);
        out
    }

    /// The same, appending to `out` so a caller reading a stream allocates no vector per line.
    pub fn push_into(&mut self, line: &str, out: &mut Vec<LogRecord>) {
        let mut rest = line;
        // The whole first line passes through verbatim; a remainder is already trimmed.
        let mut first = true;
        loop {
            let next = match self.pending.as_ref().map(|pending| pending.section) {
                None => self.outside(rest, first, out),
                Some(Section::Between) => self.between(rest, out),
                Some(_) => self.absorb(rest, out),
            };
            match next {
                Some(more) => {
                    rest = more;
                    first = false;
                }
                None => return,
            }
        }
    }

    /// Ends the stream and returns whatever a truncated event had already given up.
    pub fn flush(&mut self) -> Vec<LogRecord> {
        match self.pending.take() {
            Some(mut pending) => pending.emit(),
            None => Vec::new(),
        }
    }

    /// Handles text outside an event: an opening tag starts one, anything else is a record.
    fn outside<'a>(
        &mut self,
        rest: &'a str,
        first: bool,
        out: &mut Vec<LogRecord>,
    ) -> Option<&'a str> {
        let trimmed = rest.trim_start();
        if trimmed.starts_with(EVENT_OPEN) {
            let end = trimmed.find('>');
            let header = match end {
                Some(end) => &trimmed[..end],
                None => trimmed,
            };
            self.pending = Some(Pending::from_header(header, self.zone));
            return end.map(|end| &trimmed[end + 1..]);
        }
        if first || !trimmed.is_empty() {
            out.push(LogRecord {
                level: self.fallback,
                text: if first { rest } else { trimmed }.to_string(),
            });
        }
        None
    }

    /// Handles CDATA text, flushing the event as soon as it grows past a cap.
    fn absorb<'a>(&mut self, rest: &'a str, out: &mut Vec<LogRecord>) -> Option<&'a str> {
        let pending = self.pending.as_mut()?;
        let after = pending.absorb(rest);
        if pending.truncated && !pending.emitted {
            let records = pending.emit();
            out.extend(records);
        }
        if after.is_some() {
            pending.section = Section::Between;
        }
        after
    }

    /// Handles text between an event's elements.
    fn between<'a>(&mut self, rest: &'a str, out: &mut Vec<LogRecord>) -> Option<&'a str> {
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(after) = trimmed.strip_prefix(EVENT_CLOSE) {
            self.close(out);
            return Some(after);
        }
        // A second opening tag means the first event was never closed. Flush it and start over.
        if trimmed.starts_with(EVENT_OPEN) {
            self.close(out);
            return Some(trimmed);
        }
        for close in [MESSAGE_CLOSE, THROWABLE_CLOSE] {
            if let Some(after) = trimmed.strip_prefix(close) {
                return Some(after);
            }
        }
        for (open, section) in [
            (MESSAGE_OPEN, Section::Message),
            (THROWABLE_OPEN, Section::Throwable),
        ] {
            let Some(after) = trimmed.strip_prefix(open) else {
                continue;
            };
            let Some(body) = after.strip_prefix(CDATA_OPEN) else {
                // An element with no CDATA at all, such as an empty message.
                return Some(after);
            };
            if let Some(pending) = self.pending.as_mut() {
                pending.section = section;
            }
            return Some(body);
        }
        // Any other log4j element — properties, for one — is dropped.
        if ELEMENT_PREFIXES
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
        {
            return None;
        }
        // Plain output the game wrote between elements. It is not XML, so it passes through.
        out.push(LogRecord {
            level: self.fallback,
            text: rest.to_string(),
        });
        None
    }

    /// Ends the pending event and appends its records.
    fn close(&mut self, out: &mut Vec<LogRecord>) {
        if let Some(mut pending) = self.pending.take() {
            out.extend(pending.emit());
        }
    }
}

/// The value of one `name="value"` attribute on an element tag, with XML entities decoded.
fn attr(line: &str, name: &str) -> Option<String> {
    let start = line.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = line.get(start..)?;
    let end = rest.find('"')?;
    Some(unescape(rest.get(..end)?))
}

/// Decodes the five XML entities. `&amp;` goes last so `&amp;lt;` stays `&lt;`.
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Maps a log4j level name onto ours: `WARN` warns, `ERROR` and `FATAL` are errors, the rest
/// (`INFO`, `DEBUG`, `TRACE`, and anything unknown) is informational.
fn level_of(level: &str) -> LogLevel {
    match level {
        "WARN" => LogLevel::Warn,
        "ERROR" | "FATAL" => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// Formats a millisecond epoch timestamp as `HH:MM:SS` in `zone`, or as `HH:MM:SSZ` in UTC when
/// the zone is unknown.
fn format_time(ms: i64, zone: Option<UtcOffset>) -> Option<String> {
    let stamp = OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()?;
    match zone {
        Some(offset) => stamp.to_offset(offset).format(TIME_FORMAT).ok(),
        None => Some(format!("{}Z", stamp.format(TIME_FORMAT).ok()?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds every line of `text` through a UTC parser and returns the records it produced.
    fn convert(text: &str) -> Vec<LogRecord> {
        let mut parser = EventParser::with_offset(LogLevel::Info, UtcOffset::UTC);
        let mut out = Vec::new();
        for line in text.lines() {
            out.extend(parser.push(line));
        }
        out.extend(parser.flush());
        out
    }

    /// Just the text of every record.
    fn texts(records: &[LogRecord]) -> Vec<&str> {
        records.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn one_event_becomes_one_prefixed_line() {
        let records = convert(
            "  <log4j:Event logger=\"FabricLoader\" timestamp=\"1788800335568\" level=\"INFO\" thread=\"main\">\n\
                 <log4j:Message><![CDATA[Loading Minecraft 26.2 with Fabric Loader 0.19.5]]></log4j:Message>\n\
               </log4j:Event>\n",
        );
        assert_eq!(
            texts(&records),
            vec!["[16:58:55] [main/INFO]: Loading Minecraft 26.2 with Fabric Loader 0.19.5"]
        );
        assert_eq!(records[0].level, LogLevel::Info);
    }

    #[test]
    fn a_multi_line_message_prefixes_only_its_first_line() {
        let records = convert(
            "  <log4j:Event logger=\"FabricLoader\" timestamp=\"1788800335660\" level=\"INFO\" thread=\"main\">\n\
                 <log4j:Message><![CDATA[Loading 4 mods:\n\
             \t- fabricloader 0.19.5\n\
             \t- minecraft 26.2]]></log4j:Message>\n\
               </log4j:Event>\n",
        );
        assert_eq!(
            texts(&records),
            vec![
                "[16:58:55] [main/INFO]: Loading 4 mods:",
                "\t- fabricloader 0.19.5",
                "\t- minecraft 26.2",
            ]
        );
        assert!(records.iter().all(|r| r.level == LogLevel::Info));
    }

    #[test]
    fn a_throwable_follows_the_message_as_its_own_lines() {
        let records = convert(
            "  <log4j:Event logger=\"RealmsAvailability\" timestamp=\"1788800341884\" level=\"ERROR\" thread=\"Download-2\">\n\
                 <log4j:Message><![CDATA[Couldn't connect to realms]]></log4j:Message>\n\
                 <log4j:Throwable><![CDATA[com.mojang.realmsclient.exception.RealmsServiceException: Realms authentication error\n\
             \tat knot//com.mojang.realmsclient.client.RealmsClient.execute(RealmsClient.java:528)\n\
             ]]></log4j:Throwable>\n\
               </log4j:Event>\n",
        );
        assert_eq!(
            texts(&records),
            vec![
                "[16:59:01] [Download-2/ERROR]: Couldn't connect to realms",
                "com.mojang.realmsclient.exception.RealmsServiceException: Realms authentication error",
                "\tat knot//com.mojang.realmsclient.client.RealmsClient.execute(RealmsClient.java:528)",
            ]
        );
        assert!(records.iter().all(|r| r.level == LogLevel::Error));
    }

    #[test]
    fn a_line_outside_an_event_passes_through_unchanged() {
        let records = convert(
            "[ALSOFT] (EE) Failed to set real-time priority for thread: Operation not permitted (1)\n\
               <log4j:Event logger=\"Minecraft\" timestamp=\"1788800339730\" level=\"WARN\" thread=\"Render thread\">\n\
                 <log4j:Message><![CDATA[Missing sound for event: minecraft:item.goat_horn.play]]></log4j:Message>\n\
               </log4j:Event>\n\
             plain trailing line\n",
        );
        assert_eq!(
            texts(&records),
            vec![
                "[ALSOFT] (EE) Failed to set real-time priority for thread: Operation not permitted (1)",
                "[16:58:59] [Render thread/WARN]: Missing sound for event: minecraft:item.goat_horn.play",
                "plain trailing line",
            ]
        );
        assert_eq!(records[0].level, LogLevel::Info, "the stream's own level");
        assert_eq!(records[1].level, LogLevel::Warn, "the event's level");
    }

    #[test]
    fn a_truncated_event_is_emitted_by_flush() {
        let records = convert(
            "  <log4j:Event logger=\"Minecraft\" timestamp=\"1788800339730\" level=\"ERROR\" thread=\"main\">\n\
                 <log4j:Message><![CDATA[the process died mid-line\n\
             \tsecond line, no closing tag\n",
        );
        assert_eq!(
            texts(&records),
            vec![
                "[16:58:59] [main/ERROR]: the process died mid-line",
                "\tsecond line, no closing tag",
            ]
        );
    }

    #[test]
    fn an_event_with_no_timestamp_or_thread_still_names_its_level() {
        let records = convert(
            "<log4j:Event logger=\"x\" level=\"FATAL\">\n\
             <log4j:Message><![CDATA[boom]]></log4j:Message>\n\
             </log4j:Event>\n",
        );
        assert_eq!(texts(&records), vec!["[FATAL]: boom"]);
        assert_eq!(records[0].level, LogLevel::Error);
    }

    #[test]
    fn an_empty_message_element_yields_nothing() {
        let records = convert(
            "  <log4j:Event logger=\"x\" timestamp=\"1788800339730\" level=\"INFO\" thread=\"main\">\n\
                 <log4j:Message></log4j:Message>\n\
               </log4j:Event>\n",
        );
        assert!(records.is_empty(), "{records:?}");
    }

    #[test]
    fn the_parser_is_reusable_after_an_event_closes() {
        let mut parser = EventParser::with_offset(LogLevel::Error, UtcOffset::UTC);
        assert!(
            parser
                .push("  <log4j:Event level=\"INFO\" thread=\"main\">")
                .is_empty()
        );
        assert!(
            parser
                .push("    <log4j:Message><![CDATA[first]]></log4j:Message>")
                .is_empty()
        );
        assert_eq!(
            texts(&parser.push("  </log4j:Event>")),
            vec!["[main/INFO]: first"]
        );
        assert_eq!(
            texts(&parser.push("bare stderr line")),
            vec!["bare stderr line"]
        );
        assert!(parser.flush().is_empty());
    }

    /// The 26.2 Fabric run captured in `tests/fixtures/launch/`: one ERROR event whose throwable
    /// holds the offline account's 401 from `/player/attributes`.
    const AUTHLIB_401: &str =
        include_str!("../../../../tests/fixtures/launch/authlib_401_event.log");

    #[test]
    fn a_real_captured_event_converts_to_plain_lines() {
        let records = convert(AUTHLIB_401);
        let texts = texts(&records);
        assert_eq!(
            texts.first().copied(),
            Some("[16:58:59] [Download-2/ERROR]: Failed to fetch user properties")
        );
        assert_eq!(
            texts.get(1).copied(),
            Some("com.mojang.authlib.exceptions.InvalidCredentialsException: Status: 401")
        );
        assert_eq!(texts.last().copied(), Some("\t... 5 more"), "{texts:?}");
        assert!(
            texts.iter().any(|line| line.starts_with("Caused by: ")),
            "{texts:?}"
        );
        assert!(
            !texts
                .iter()
                .any(|line| line.contains("log4j") || line.contains("CDATA")),
            "{texts:?}"
        );
        assert!(records.iter().all(|r| r.level == LogLevel::Error));
    }

    #[test]
    fn a_throwable_that_starts_where_the_message_ends_is_kept() {
        let records = convert(
            "<log4j:Event timestamp=\"1788800339730\" level=\"ERROR\" thread=\"main\">\n\
             <log4j:Message><![CDATA[boom]]></log4j:Message><log4j:Throwable><![CDATA[java.lang.Exception: boom\n\
             \tat Main.main(Main.java:1)]]></log4j:Throwable>\n\
             </log4j:Event>\n",
        );
        assert_eq!(
            texts(&records),
            vec![
                "[16:58:59] [main/ERROR]: boom",
                "java.lang.Exception: boom",
                "\tat Main.main(Main.java:1)",
            ]
        );
    }

    #[test]
    fn an_escaped_cdata_close_stays_part_of_the_message() {
        // log4j writes a literal `]]>` as `]]]]><![CDATA[>`.
        let records = convert(
            "<log4j:Event level=\"INFO\" thread=\"main\">\n\
             <log4j:Message><![CDATA[a]]]]><![CDATA[>b]]></log4j:Message>\n\
             </log4j:Event>\n",
        );
        assert_eq!(texts(&records), vec!["[main/INFO]: a]]>b"]);
    }

    #[test]
    fn a_whole_event_on_one_line_yields_its_record() {
        let records = convert(
            "<log4j:Event logger=\"x\" timestamp=\"1788800339730\" level=\"WARN\" thread=\"main\"><log4j:Message><![CDATA[x]]></log4j:Message></log4j:Event>\n",
        );
        assert_eq!(texts(&records), vec!["[16:58:59] [main/WARN]: x"]);
        assert_eq!(records[0].level, LogLevel::Warn);
    }

    #[test]
    fn a_plain_line_inside_an_event_passes_through_at_the_stream_level() {
        let mut parser = EventParser::with_offset(LogLevel::Error, UtcOffset::UTC);
        assert!(
            parser
                .push("<log4j:Event level=\"INFO\" thread=\"main\">")
                .is_empty()
        );
        let stray = parser.push("[ALSOFT] (EE) Failed to set real-time priority");
        assert_eq!(
            texts(&stray),
            vec!["[ALSOFT] (EE) Failed to set real-time priority"]
        );
        assert_eq!(stray[0].level, LogLevel::Error, "the stream's own level");
        assert!(
            parser
                .push("<log4j:Message><![CDATA[after]]></log4j:Message>")
                .is_empty()
        );
        assert_eq!(
            texts(&parser.push("</log4j:Event>")),
            vec!["[main/INFO]: after"]
        );
    }

    #[test]
    fn a_new_event_flushes_the_one_still_open() {
        let mut parser = EventParser::with_offset(LogLevel::Info, UtcOffset::UTC);
        assert!(
            parser
                .push("<log4j:Event level=\"INFO\" thread=\"main\">")
                .is_empty()
        );
        assert!(
            parser
                .push("<log4j:Message><![CDATA[first, never closed]]></log4j:Message>")
                .is_empty()
        );
        let records = parser.push("<log4j:Event level=\"ERROR\" thread=\"other\">");
        assert_eq!(texts(&records), vec!["[main/INFO]: first, never closed"]);
        assert!(
            parser
                .push("<log4j:Message><![CDATA[second]]></log4j:Message>")
                .is_empty()
        );
        let records = parser.push("</log4j:Event>");
        assert_eq!(texts(&records), vec!["[other/ERROR]: second"]);
        assert_eq!(records[0].level, LogLevel::Error);
    }

    #[test]
    fn a_runaway_event_is_capped_and_flushed_once() {
        let mut parser = EventParser::with_offset(LogLevel::Info, UtcOffset::UTC);
        parser.push("<log4j:Event level=\"INFO\" thread=\"main\">");
        parser.push("<log4j:Message><![CDATA[start of a body that never closes");
        let mut count = 0;
        let mut last = String::new();
        for i in 0..200_000 {
            for record in parser.push(&format!("line {i}")) {
                count += 1;
                last = record.text;
            }
        }
        assert_eq!(count, MAX_EVENT_LINES + 1, "one flush of the capped body");
        assert_eq!(last, TRUNCATION_MARK);
        assert!(parser.flush().is_empty(), "nothing is still buffered");
    }

    #[test]
    fn an_unknown_local_offset_marks_the_timestamp_as_utc() {
        let mut parser = EventParser::with_zone(LogLevel::Info, None);
        parser.push("<log4j:Event timestamp=\"1788800339730\" level=\"INFO\" thread=\"main\">");
        parser.push("<log4j:Message><![CDATA[x]]></log4j:Message>");
        let records = parser.push("</log4j:Event>");
        assert_eq!(texts(&records), vec!["[16:58:59Z] [main/INFO]: x"]);
    }

    #[test]
    fn a_known_local_offset_renders_the_timestamp_in_it() {
        let plus_two = UtcOffset::from_hms(2, 0, 0).expect("a valid offset");
        let mut parser = EventParser::with_zone(LogLevel::Info, Some(plus_two));
        parser.push("<log4j:Event timestamp=\"1788800339730\" level=\"INFO\" thread=\"main\">");
        parser.push("<log4j:Message><![CDATA[x]]></log4j:Message>");
        let records = parser.push("</log4j:Event>");
        assert_eq!(texts(&records), vec!["[18:58:59] [main/INFO]: x"]);
    }

    #[test]
    fn init_local_offset_fills_the_lock() {
        init_local_offset();
        assert!(LOCAL_OFFSET.get().is_some(), "the lock is filled");
    }

    #[test]
    fn xml_entities_in_an_attribute_are_decoded() {
        let mut parser = EventParser::with_offset(LogLevel::Info, UtcOffset::UTC);
        parser.push("<log4j:Event level=\"INFO\" thread=\"Netty &lt;Client&gt; IO #0 &amp;co\">");
        parser.push("<log4j:Message><![CDATA[x]]></log4j:Message>");
        let records = parser.push("</log4j:Event>");
        assert_eq!(texts(&records), vec!["[Netty <Client> IO #0 &co/INFO]: x"]);
    }
}
