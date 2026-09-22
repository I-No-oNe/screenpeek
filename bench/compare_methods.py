#!/usr/bin/env python3
"""Compare live observations; distinct coordinates are not click-accuracy scores."""
import argparse
import json
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def clickable(elements):
    """Count unique names and their distinct coordinates.

    >>> clickable([{"text": "A", "x": 1, "y": 2}, {"text": "B", "x": 1, "y": 2}])
    (2, 1)
    >>> clickable([{"text": "A", "x": 1, "y": 2}, {"text": "A", "x": 3, "y": 4}])
    (0, 0)
    """
    places = {}
    for element in elements:
        places.setdefault(element["text"], []).append((element["x"], element["y"]))
    points = [spots[0] for spots in places.values() if len(spots) == 1]
    return len(points), len(set(points))


def timed(command, output=None):
    started = time.perf_counter()
    result = subprocess.run(command, capture_output=True, text=True, timeout=300)
    elapsed = (time.perf_counter() - started) * 1000
    if result.returncode:
        raise SystemExit(f"{command[0]} failed: {result.stderr.strip()}")
    return json.loads(output.read_text() if output else result.stdout), elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--reader", choices=["dogtail", "atspi-reference"], default="dogtail")
    parser.add_argument("--output", type=Path, default=ROOT / "target/methods.json")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    samples = {args.reader: [], "screenpeek": []}
    with tempfile.TemporaryDirectory(prefix="screenpeek-comparison-") as directory:
        output = Path(directory) / "dogtail.json"
        if args.reader == "dogtail":
            command = [sys.executable, str(ROOT / "bench/dogtail_runner.py"), str(output)]
        else:
            command = [sys.executable, str(ROOT / "bench/atspi_reference.py")]
            output = None
        for _ in range(args.runs):
            samples[args.reader].append(timed(command, output))
            samples["screenpeek"].append(timed([str(ROOT / "target/release/screenpeek"), "scan", "--json"]))
    report = {}
    for name, runs in samples.items():
        elements = runs[-1][0]
        named, distinct = clickable(elements)
        report[name] = {"elements": len(elements), "unique_names": named,
                        "distinct_points": distinct, "median_ms": round(statistics.median(t for _, t in runs), 1),
                        "samples_ms": [round(t, 1) for _, t in runs]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    print("Observation timings only; coordinate uniqueness does not establish successful clicks.", file=sys.stderr)


if __name__ == "__main__":
    main()
