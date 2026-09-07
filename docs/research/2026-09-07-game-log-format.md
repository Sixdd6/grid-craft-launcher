# The game log: log4j XML and the crash hint

Source: a real 26.2 Fabric run captured on 2026-09-07 into `<root>/logs/demo-*.log`. A short
excerpt of it is committed as `tests/fixtures/launch/authlib_401_event.log`.

## What the game writes

Mojang's `logging.client` config makes the game print one XML element per event on stdout:

```xml
  <log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1788800339730" level="ERROR" thread="Download-2">
    <log4j:Message><![CDATA[Failed to fetch user properties]]></log4j:Message>
    <log4j:Throwable><![CDATA[com.mojang.authlib.exceptions.InvalidCredentialsException: Status: 401
	at knot//...
]]></log4j:Throwable>
  </log4j:Event>
```

Facts the parser (`gcl-core/src/launch/log4j.rs`) depends on, all seen in the capture:

- `timestamp` is milliseconds since the epoch. `level` is `TRACE`..`FATAL`. `thread` and
  `logger` are free text; attribute values carry the five XML entities, so they are decoded.
- One physical line can hold several elements: a whole event, or a `<log4j:Throwable>` that
  starts where `</log4j:Message>` ended. The parser walks the remainder of a line after every
  `]]>` and every tag, so nothing on the line is lost.
- A literal `]]>` inside a body is written `]]]]><![CDATA[>`. Text after `]]>` that opens CDATA
  again continues the same body.
- Not everything on stdout is XML. Native libraries (`[ALSOFT] (EE) …`) and stderr write plain
  lines, before an event, between two events, and between the elements of one event. Every such
  line passes through at the stream's own level.
- An event can be cut short: the process dies, or a second `<log4j:Event` opens before the first
  closes. Both flush what the first event already had.
- One event cannot be trusted to end. The parser buffers at most 20 000 lines or 1 MiB per
  event, then appends one `… (truncated)` line, flushes, and stops buffering.

Converted output is the shape a vanilla launcher shows: `[HH:MM:SS] [thread/LEVEL]: message`,
then every later message line and every throwable line verbatim. The clock is local time, read
once by `launch::init_local_offset()` in `main` before any thread starts — `time` refuses to
read the offset from a threaded process. When it stays unknown the time is UTC, marked `Z`.

## The crash hint

`launch::crash_hint` quotes one line of the last 200 as the reason for a non-zero exit. Markers,
most telling first: `Caused by`, `Mod File:`, `Mixin`, `Exception`.

Two rules pick the line, both from this capture:

- **Recency.** Each marker is searched from the end backwards. Every offline account gets a
  boot-time `Caused by: MinecraftClientHttpException[… status=401 … path=/player/attributes …]`
  seconds after the window opens. Searching forwards made that line the hint for every crash in
  the run. Searching backwards lets a later crash win.
- **Level.** For `Mixin` and `Exception` an INFO line is skipped, read off the converted
  `[HH:MM:SS] [thread/LEVEL]:` prefix. Fabric's mixin subsystem logs at INFO all through boot.

A run whose only marker is that 401 still reports it: it is the one failure the log names, and
it is logged at ERROR.
