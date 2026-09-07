import sys, time, subprocess
from Xlib import display, X
from Xlib.ext import xtest
d = display.Display(":99")
def click(x, y, hold=0.05):
    xtest.fake_input(d, X.MotionNotify, x=x, y=y); d.sync(); time.sleep(0.15)
    xtest.fake_input(d, X.ButtonPress, 1); d.sync(); time.sleep(hold)
    xtest.fake_input(d, X.ButtonRelease, 1); d.sync(); time.sleep(0.4)
def key(keysym_name):
    from Xlib import XK
    ks = XK.string_to_keysym(keysym_name); kc = d.keysym_to_keycode(ks)
    xtest.fake_input(d, X.KeyPress, kc); d.sync(); time.sleep(0.03)
    xtest.fake_input(d, X.KeyRelease, kc); d.sync(); time.sleep(0.08)
def typ(s):
    for ch in s:
        name = {' ': 'space'}.get(ch, ch)
        key(name)
def shot(name):
    subprocess.run(["import", "-display", ":99", "-window", "root", name], check=True)
steps = sys.argv[1:]
for step in steps:
    kind, _, arg = step.partition(":")
    if kind == "click": x, y = map(int, arg.split(",")); click(x, y)
    elif kind == "type": typ(arg)
    elif kind == "key": key(arg)
    elif kind == "drag":
        x1,y1,x2,y2 = map(int, arg.split(","))
        xtest.fake_input(d, X.MotionNotify, x=x1, y=y1); d.sync(); time.sleep(0.15)
        xtest.fake_input(d, X.ButtonPress, 1); d.sync(); time.sleep(0.1)
        for i in range(1,11):
            xtest.fake_input(d, X.MotionNotify, x=x1+(x2-x1)*i//10, y=y1+(y2-y1)*i//10); d.sync(); time.sleep(0.04)
        xtest.fake_input(d, X.ButtonRelease, 1); d.sync(); time.sleep(0.8)
    elif kind == "sleep": time.sleep(float(arg))
    elif kind == "shot": shot(arg)
# drag support
