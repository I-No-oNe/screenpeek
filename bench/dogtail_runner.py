#!/usr/bin/env python3
"""Export dogtail names and extents for observation comparisons."""
import json
import os
import sys
import time

os.environ.setdefault("GTK_MODULES", "gail:atk-bridge")

import gi  # noqa: E402

gi.require_version("Atspi", "2.0")

from dogtail import tree  # noqa: E402
from dogtail.config import config  # noqa: E402

config.searchShowingOnly = False
config.logDebugToFile = False
config.logDebugToStdOut = False


def collect(node, out, depth=0):
    if depth > 40 or len(out) > 20000:
        return
    try:
        name = (node.name or "").strip()
        extents = node.extents
    except Exception:
        name, extents = "", None
    if name and extents and extents[2] > 0 and extents[3] > 0:
        out.append({
            "text": name,
            "x": extents[0] + extents[2] // 2,
            "y": extents[1] + extents[3] // 2,
            "width": extents[2],
            "height": extents[3],
            "role": str(getattr(node, "roleName", "")),
        })
    try:
        children = node.children
    except Exception:
        return
    for child in children:
        collect(child, out, depth + 1)


def main():
    started = time.perf_counter()
    found = []
    for app in tree.root.applications():
        collect(app, found)
    elapsed = (time.perf_counter() - started) * 1000

    for index, element in enumerate(found):
        element["id"] = index
    # dogtail writes its own notes to stdout, so the results go to a file.
    out = sys.argv[1] if len(sys.argv) > 1 else "/dev/stdout"
    with open(out, "w") as handle:
        json.dump(found, handle, ensure_ascii=False, indent=2)
    print(f"dogtail: {len(found)} elements in {elapsed:.0f} ms", file=sys.stderr)


if __name__ == "__main__":
    main()
