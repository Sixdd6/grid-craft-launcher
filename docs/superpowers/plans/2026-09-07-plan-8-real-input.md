# Plan 8: real-input fixes

Date: 2026-09-07. Status: merged pending. Follows plan 7.

## Why

The user reported dead and unresponsive buttons after plan 7. The headless flow tests fire
callbacks through accessible actions, which skip pointer hit-testing. Driving the real window
with XTest on Xvfb (`scripts/ui-xtest.py`) reproduced these defects:

1. The first click after a dialog closes with Escape is lost. Closing with the Cancel button
   does not lose it. Every dialog stays mounted while closed.
2. Number shortcuts stop working after a screen change: the focused field is destroyed with its
   screen and no element takes the keyboard back.
3. Preview defaults leak into the running app: the browser query starts as "sodium", the Logs
   tab of a never-launched instance shows two placeholder lines, and every global carries sample
   rows until its first load.
4. Minecraft writes log4j XML to stdout, so the Logs tab and the crash hint show raw XML.
5. The catalog stores `fov` as degrees and unquoted `renderClouds`/`mainHand` tokens. A real
   26.2 `options.txt` stores `fov:0.0` (float, -1..1, 0 = 70 degrees) and `renderClouds:"true"`.
6. The Stop button is painted red while disabled, so it reads as clickable.

Verified working with real input: create dialog (name, version and loader combos, Create),
loader install, instance tabs, Check updates, Add content, settings slider drag, switch, choice
popup, JVM tab, Rename prompt, Accounts, Settings, Launch (downloads, starts the game, Stop
enabled), keyboard once the window has focus, Wayland rendering under a nested KWin.

## Tasks

1. core: parse log4j events into plain lines; catalog fixes with display scaling for `fov`.
2. ui: mount dialogs only while open; refocus the rail on screen change; Stop visible only while
   running; harness clicks through pointer events; verify with `scripts/ui-xtest.py`.
3. ui: empty defaults in every global; `Preview*` components carry the sample data.
4. docs, skills, e2e, merge.

## Verified

The final `just ui-xtest` pass, over the merged fix commits, covered:

- Creating an instance through the Create dialog (name, version and loader combos, Create).
- A never-launched instance's Logs tab shows no log lines and no sample status.
- The click right after closing a dialog with Escape still lands on the next element, instead of
  being eaten by a dialog that stayed mounted.
- A number-key shortcut still reaches the navigation rail after a screen change that passed
  through a focused text field.
- The browser opens with an empty search box and no result rows, instead of a leftover preview
  query and hits.
