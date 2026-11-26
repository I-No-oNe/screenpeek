#!/usr/bin/env python3
"""Time screenpeek actions on the live desktop: p50 and p95 per command.

Usage: bench/actions.py [--runs N] [--binary PATH] -- 'click 3 --check' 'run "focus B"' ...

Each argument is one command line, run --runs times in turn with the others,
so a command that toggles state (focus A, focus B) can be paired with its undo.

>>> percentile([5, 1, 3, 2, 4], 50)
3
>>> percentile([1, 2, 3, 4, 100], 95)
100
"""

import argparse
import json
import shlex
import subprocess
import sys
import time
from pathlib import Path


def percentile(values, rank):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(rank / 100 * (len(ordered) - 1)))]


def main():
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--runs", type=int, default=20)
    parser.add_argument("--binary", default=str(root / "target/release/screenpeek"))
    parser.add_argument("commands", nargs="+")
    args = parser.parse_args()

    times = {command: [] for command in args.commands}
    for _ in range(args.runs):
        for command in args.commands:
            started = time.perf_counter()
            done = subprocess.run([args.binary, *shlex.split(command)], capture_output=True, text=True)
            times[command].append((time.perf_counter() - started) * 1000)
            if done.returncode != 0:
                sys.exit(f"{command!r} failed: {done.stderr.strip()}")

    report = {
        command: {"p50_ms": round(percentile(values, 50)), "p95_ms": round(percentile(values, 95))}
        for command, values in times.items()
    }
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
