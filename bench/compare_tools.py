#!/usr/bin/env python3
"""Compare OCR on identical images with labelled target rectangles (no desktop actions)."""
import argparse
import csv
import io
import json
import os
from pathlib import Path

from measure import ROOT, measure, run


def tesseract_elements(output):
    lines = {}
    for row in csv.DictReader(io.StringIO(output), delimiter="\t"):
        if row["level"] != "5" or not row["text"].strip():
            continue
        key = tuple(row[k] for k in ("page_num", "block_num", "par_num", "line_num"))
        left, top, width, height = (int(row[k]) for k in ("left", "top", "width", "height"))
        line = lines.setdefault(key, dict(words=[], left=left, top=top, right=left, bottom=top))
        line["words"].append(row["text"])
        line["left"], line["top"] = min(line["left"], left), min(line["top"], top)
        line["right"], line["bottom"] = max(line["right"], left + width), max(line["bottom"], top + height)
    return [dict(text=" ".join(l["words"]), x=(l["left"] + l["right"]) / 2,
                 y=(l["top"] + l["bottom"]) / 2) for l in lines.values()]


def score(elements, targets):
    """Count a label only when its point is unambiguous and inside its target.

    >>> target = dict(label="Save", x=10, y=20, width=30, height=10)
    >>> hit = dict(text="Save", x=25, y=25)
    >>> score([hit], [target])["correct_coordinates"]
    1.0
    >>> score([hit, hit], [target])["resolution"]
    0.0
    >>> score([dict(hit, x=40)], [target])["correct_coordinates"]
    0.0
    """
    resolved = correct = 0
    for target in targets:
        hits = [e for e in elements if e["text"] == target["label"]]
        # Ambiguous labels cannot safely resolve a named click.
        resolved += len(hits) == 1
        correct += len(hits) == 1 and (target["x"] <= hits[0]["x"] < target["x"] + target["width"]
                    and target["y"] <= hits[0]["y"] < target["y"] + target["height"])
    return dict(resolution=resolved / len(targets), correct_coordinates=correct / len(targets))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifests", type=Path, nargs="*", default=[ROOT / "bench/fixtures/dialog.json"])
    parser.add_argument("--runs", type=int, default=7)
    args = parser.parse_args()
    if args.runs < 3:
        parser.error("use at least 3 runs")
    binary = str(Path(os.environ.get("SCREENPEEK", ROOT / "target/release/screenpeek")).resolve())
    report = []
    for path in args.manifests:
        fixture = json.loads(path.read_text())
        image = str(path.parent / fixture["image"])
        language = fixture.get("language") or "eng"
        runners = [
            ("screenpeek builtin", [binary, "read", image, "--json"], json.loads),
            ("screenpeek tesseract", [binary, "read", image, "--json", "--lang", language], json.loads),
            ("tesseract", ["tesseract", image, "stdout", "-l", language, "--psm", "11", "-c", "tessedit_create_tsv=1"], tesseract_elements),
        ]
        for name, command, parse in runners:
            run(command)
            timing, samples = measure(command, args.runs)
            scores = [score(parse(output), fixture["targets"]) for _, output in samples]
            report.append(dict(fixture=path.stem, tool=name, **timing,
                               **{key: min(s[key] for s in scores) for key in scores[0]}))
    output = ROOT / "target/compare.json"
    output.parent.mkdir(exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print("| Fixture | Tool | Exact resolution | Correct coordinates | Median ms |")
    print("| --- | --- | ---: | ---: | ---: |")
    for row in report:
        print(f"| {row['fixture']} | {row['tool']} | {row['resolution']:.0%} | {row['correct_coordinates']:.0%} | {row['median_ms']} |")


if __name__ == "__main__":
    main()
