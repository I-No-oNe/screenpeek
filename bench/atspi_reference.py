#!/usr/bin/env python3
"""Diagnostic AT-SPI reader; its results are not dogtail measurements."""
import argparse
import json
import subprocess
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

# A tree can be deep and a walk can wander; dogtail has the same problem and
# the same kind of guard.
MAX_NODES = 20000
MAX_DEPTH = 40


def walk(node, out, depth=0, seen=0):
    if depth > MAX_DEPTH or len(out) > MAX_NODES:
        return
    # Continue into children when an application node has no extents.
    try:
        name = (node.get_name() or "").strip()
    except Exception:
        name = ""
    try:
        extents = node.get_extents(Atspi.CoordType.SCREEN)
    except Exception:
        extents = None
    if name and extents is not None and extents.width > 0 and extents.height > 0:
        try:
            role = node.get_role_name()
        except Exception:
            role = ""
        out.append({
            "text": name,
            "x": extents.x + extents.width // 2,
            "y": extents.y + extents.height // 2,
            "width": extents.width,
            "height": extents.height,
            "role": role,
        })
    try:
        count = node.get_child_count()
    except Exception:
        return
    for i in range(count):
        try:
            child = node.get_child_at_index(i)
        except Exception:
            continue
        if child is not None:
            walk(child, out, depth + 1)


def windows():
    """Where the compositor says each window is.

    On Wayland a client does not know its own position, so AT-SPI reports
    extents relative to the window however `SCREEN` coordinates are asked
    for. A tool that wants to click has to ask the compositor and add the
    offset -- which is what screenpeek's fuse step does, and what this gives
    the accessibility method so the comparison is about the method and not
    about who knows this trick.
    """
    try:
        listed = subprocess.run(["hyprctl", "clients", "-j"], capture_output=True,
                                text=True, timeout=30, check=True).stdout
    except Exception:
        return []
    return [{"title": c.get("title", ""), "x": c["at"][0], "y": c["at"][1],
             "width": c["size"][0], "height": c["size"][1]}
            for c in json.loads(listed)]


def place(found, frames):
    """Offsets each window's nodes by where the compositor put that window."""
    places = windows()
    for frame in frames:
        # Title first, then size: a tiled desktop gives two windows the same
        # size, so size alone picks the wrong one.
        fits = lambda w: (abs(w["width"] - frame["width"]) <= 2
                          and abs(w["height"] - frame["height"]) <= 2)
        match = next((w for w in places if w["title"] == frame["text"] and fits(w)), None)
        match = match or next((w for w in places if w["title"] == frame["text"]), None)
        match = match or next((w for w in places if fits(w)), None)
        if match is None:
            continue
        places = [w for w in places if w is not match]
        left = frame["x"] - frame["width"] // 2
        top = frame["y"] - frame["height"] // 2
        for element in found[frame["first"]:frame["last"]]:
            element["x"] += match["x"] - left
            element["y"] += match["y"] - top


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, help="only this application")
    parser.add_argument("--raw", action="store_true",
                        help="leave coordinates as AT-SPI reports them")
    args = parser.parse_args()

    # The registry has to be connected before the desktop has any children,
    # which is what dogtail's own import does for you.
    Atspi.init()
    started = time.perf_counter()
    desktop = Atspi.get_desktop(0)
    found = []
    frames = []
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        if app is None:
            continue
        if args.pid is not None:
            try:
                if app.get_process_id() != args.pid:
                    continue
            except Exception:
                continue
        first = len(found)
        walk(app, found)
        window = next((e for e in found[first:] if e.get("role") == "frame"), None)
        if window is not None:
            frames.append(dict(window, first=first, last=len(found)))
    if not args.raw:
        place(found, frames)
    elapsed = (time.perf_counter() - started) * 1000

    for index, element in enumerate(found):
        element["id"] = index
    print(json.dumps(found, ensure_ascii=False, indent=2))
    print(f"{len(found)} elements in {elapsed:.0f} ms", file=sys.stderr)


if __name__ == "__main__":
    main()
