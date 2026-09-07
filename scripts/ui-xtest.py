#!/usr/bin/env python3
"""Drive the running GUI with real X input, through the XTest extension.

Every step is one argument, applied in order:

    focus[:wm-class]   give the keyboard to the app window (default class
                       `grid-craft-launcher`); needed once before any `type` or `key`
    click:X,Y          move the pointer there, press button 1, release it
    drag:X1,Y1,X2,Y2   press at the first point, move in ten steps, release at the second
    type:TEXT          type the text, one key at a time
    key:NAME           press one key by keysym name, e.g. `Escape` or `Return`
    sleep:SECONDS      wait
    shot:PATH          save a PNG of the root window

The display comes from `GCL_XTEST_DISPLAY`, else `DISPLAY`. The events go to the X server,
not to the toolkit, so they take the same path a mouse and a keyboard do: hit-testing,
pointer grabs, and focus all behave as they do for a user.
"""

import os
import subprocess
import sys
import time

from Xlib import X, XK, display
from Xlib.ext import xtest

DISPLAY = os.environ.get("GCL_XTEST_DISPLAY") or os.environ.get("DISPLAY") or ":99"
d = display.Display(DISPLAY)


def click(x, y, hold=0.05):
    xtest.fake_input(d, X.MotionNotify, x=x, y=y)
    d.sync()
    time.sleep(0.15)
    xtest.fake_input(d, X.ButtonPress, 1)
    d.sync()
    time.sleep(hold)
    xtest.fake_input(d, X.ButtonRelease, 1)
    d.sync()
    time.sleep(0.4)


def key(keysym_name):
    ks = XK.string_to_keysym(keysym_name)
    kc = d.keysym_to_keycode(ks)
    xtest.fake_input(d, X.KeyPress, kc)
    d.sync()
    time.sleep(0.03)
    xtest.fake_input(d, X.KeyRelease, kc)
    d.sync()
    time.sleep(0.08)


def typ(s):
    for ch in s:
        name = {" ": "space"}.get(ch, ch)
        key(name)


def drag(x1, y1, x2, y2):
    xtest.fake_input(d, X.MotionNotify, x=x1, y=y1)
    d.sync()
    time.sleep(0.15)
    xtest.fake_input(d, X.ButtonPress, 1)
    d.sync()
    time.sleep(0.1)
    for i in range(1, 11):
        xtest.fake_input(
            d, X.MotionNotify, x=x1 + (x2 - x1) * i // 10, y=y1 + (y2 - y1) * i // 10
        )
        d.sync()
        time.sleep(0.04)
    xtest.fake_input(d, X.ButtonRelease, 1)
    d.sync()
    time.sleep(0.8)


def windows_of(window, wanted):
    """Every window under `window` whose WM_CLASS names `wanted`, deepest last."""
    found = []
    try:
        cls = window.get_wm_class()
    except Exception:
        cls = None
    if cls and any(wanted in part for part in cls):
        found.append(window)
    try:
        children = window.query_tree().children
    except Exception:
        children = []
    for child in children:
        found.extend(windows_of(child, wanted))
    return found


def focus(wm_class="grid-craft-launcher"):
    """Give the keyboard to the app window.

    A bare Xvfb runs no window manager, so nothing hands out the input focus and every key
    goes to the root window. The app never sees a key press until this runs.
    """
    deadline = time.time() + 20
    while time.time() < deadline:
        found = windows_of(d.screen().root, wm_class)
        if found:
            window = found[-1]
            window.set_input_focus(X.RevertToParent, X.CurrentTime)
            window.configure(stack_mode=X.Above)
            d.sync()
            time.sleep(0.3)
            return
        time.sleep(0.5)
    raise SystemExit(f"no window with WM_CLASS `{wm_class}` on {DISPLAY}")


def shot(name):
    subprocess.run(
        ["import", "-display", DISPLAY, "-window", "root", name], check=True
    )


def main(steps):
    for step in steps:
        kind, _, arg = step.partition(":")
        if kind == "click":
            x, y = map(int, arg.split(","))
            click(x, y)
        elif kind == "type":
            typ(arg)
        elif kind == "key":
            key(arg)
        elif kind == "focus":
            focus(arg or "grid-craft-launcher")
        elif kind == "drag":
            drag(*map(int, arg.split(",")))
        elif kind == "sleep":
            time.sleep(float(arg))
        elif kind == "shot":
            shot(arg)
        else:
            raise SystemExit(f"unknown step `{step}`")


if __name__ == "__main__":
    main(sys.argv[1:])
