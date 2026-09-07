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
    /// Between elements: the next line opens a message, a throwable, or closes the event.
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
}

impl Pending {
    /// Reads the attributes off an opening `<log4j:Event …>` line.
    fn from_header(line: &str, offset: UtcOffset) -> Self {
        let level_text = attr(line, "level").unwrap_or("INFO").to_uppercase();
        Self {
            level: level_of(&level_text),
            level_text,
            thread: attr(line, "thread").unwrap_or_default().to_string(),
            time: attr(line, "timestamp")
                .and_then(|ms| ms.parse::<i64>().ok())
                .and_then(|ms| format_time(ms, offset)),
            message: Vec::new(),
            throwable: Vec::new(),
            section: Section::Between,
        }
    }

    /// Reads an element opening inside the event, and stays in it if its CDATA runs on.
    ///
    /// Any other element (properties, for one) is dropped.
    fn open_element(&mut self, trimmed: &str) {
        if let Some(rest) = body_after(trimmed, MESSAGE_OPEN) {
            let ended = absorb(&mut self.message, rest, MESSAGE_CLOSE);
            self.section = if ended {
                Section::Between
            } else {
                Section::Message
            };
        } else if let Some(rest) = body_after(trimmed, THROWABLE_OPEN) {
            let ended = absorb(&mut self.throwable, rest, THROWABLE_CLOSE);
            self.section = if ended {
                Section::Between
            } else {
                Section::Throwable
            };
        }
    }

    /// The plain lines this event becomes.
    ///
    /// The first message line carries the `[HH:MM:SS] [thread/LEVEL]: ` prefix. Every later
    /// message line and every throwable line is its own record at the same level, verbatim, so
    /// a stack trace keeps its leading tabs.
    fn records(self) -> Vec<LogRecord> {
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
        let mut out = Vec::with_capacity(self.message.len() + self.throwable.len());
        let mut message = self.message.into_iter();
        if let Some(first) = message.next() {
            out.push(LogRecord {
                level,
                text: format!("{stamp}[{source}]: {first}"),
            });
        }
        for line in message.chain(self.throwable) {
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
    /// The offset event timestamps are shown in.
    offset: UtcOffset,
    /// The event being read, if any.
    pending: Option<Pending>,
}

impl EventParser {
    /// A parser that shows timestamps in local time and reports plain lines at `fallback`.
    pub fn new(fallback: LogLevel) -> Self {
        Self::with_offset(fallback, local_offset())
    }

    /// The same, with the time zone given rather than the machine's. For tests.
    pub fn with_offset(fallback: LogLevel, offset: UtcOffset) -> Self {
        Self {
            fallback,
            offset,
            pending: None,
        }
    }

    /// Feeds one line and returns the plain lines it completed, if any.
    pub fn push(&mut self, line: &str) -> Vec<LogRecord> {
        match self.pending.as_mut() {
            None => {
                if line.trim_start().starts_with(EVENT_OPEN) {
                    self.pending = Some(Pending::from_header(line, self.offset));
                    Vec::new()
                } else {
                    vec![LogRecord {
                        level: self.fallback,
                        text: line.to_string(),
                    }]
                }
            }
            Some(pending) => {
                match pending.section {
                    Section::Message => {
                        if absorb(&mut pending.message, line, MESSAGE_CLOSE) {
                            pending.section = Section::Between;
                        }
                    }
                    Section::Throwable => {
                        if absorb(&mut pending.throwable, line, THROWABLE_CLOSE) {
                            pending.section = Section::Between;
                        }
                    }
                    Section::Between => {
                        let trimmed = line.trim_start();
                        if trimmed.starts_with(EVENT_CLOSE) {
                            let done = self.pending.take();
                            return done.map(Pending::records).unwrap_or_default();
                        }
                        pending.open_element(trimmed);
                    }
                }
                Vec::new()
            }
        }
    }

    /// Ends the stream and returns whatever a truncated event had already given up.
    pub fn flush(&mut self) -> Vec<LogRecord> {
        self.pending
            .take()
            .map(Pending::records)
            .unwrap_or_default()
    }
}

/// Adds `chunk` to a CDATA body and says whether the element ended on this line.
///
/// `]]>` closes the body; everything before it belongs to the body. A closing tag with no CDATA
/// at all (an empty message) ends the body and adds nothing.
fn absorb(buf: &mut Vec<String>, chunk: &str, close_tag: &str) -> bool {
    if let Some(end) = chunk.find(CDATA_CLOSE) {
        buf.push(chunk[..end].to_string());
        return true;
    }
    if chunk.trim_start().starts_with(close_tag) {
        return true;
    }
    buf.push(chunk.to_string());
    false
}

/// The text after an opening element tag and its `<![CDATA[`, if the line opens that element.
fn body_after<'a>(trimmed: &'a str, open_tag: &str) -> Option<&'a str> {
    let rest = trimmed.strip_prefix(open_tag)?;
    Some(rest.strip_prefix(CDATA_OPEN).unwrap_or(rest))
}

/// The value of one `name="value"` attribute on an element line.
fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let start = line.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = line.get(start..)?;
    let end = rest.find('"')?;
    rest.get(..end)
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

/// Formats a millisecond epoch timestamp as `HH:MM:SS` in `offset`.
fn format_time(ms: i64, offset: UtcOffset) -> Option<String> {
    let stamp = OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()?;
    stamp.to_offset(offset).format(TIME_FORMAT).ok()
}

/// The machine's UTC offset, read once.
///
/// `time` refuses to read the local offset from a process that has already started threads, and
/// the launcher has. UTC then stands in, so a timestamp is always shown, never dropped.
fn local_offset() -> UtcOffset {
    static OFFSET: OnceLock<UtcOffset> = OnceLock::new();
    *OFFSET.get_or_init(|| match UtcOffset::current_local_offset() {
        Ok(offset) => offset,
        Err(_) => UtcOffset::UTC,
    })
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
                "",
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
}
