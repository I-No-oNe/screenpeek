#!/usr/bin/env python3
"""Estimate labelled-action savings; model latency is an assumption, never a measurement."""
import argparse
import json
import math
import os
from pathlib import Path
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "https://developers.openai.com/api/docs/guides/images-vision#tile-based-image-tokenization"


def image_tokens(width, height):
    """GPT-4.1 high-detail sizing, per OpenAI documentation.

    >>> image_tokens(900, 560)
    765
    >>> image_tokens(1920, 1080)
    1105
    >>> image_tokens(340, 60)
    255
    """
    scale = min(1, 2048 / max(width, height))
    width, height = math.floor(width * scale), math.floor(height * scale)
    scale = min(1, 768 / min(width, height))
    width, height = math.floor(width * scale), math.floor(height * scale)
    return 85 + 170 * math.ceil(width / 512) * math.ceil(height / 512)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vision-ms", type=float, nargs="+", default=[1000, 3000, 5000],
                        help="assumed extra model round-trip times")
    parser.add_argument("--overhead-ms", type=float, default=30,
                        help="assumed shared capture, IPC/process and click overhead")
    parser.add_argument("--scenarios", type=Path, default=ROOT / "target/scenarios.json")
    args = parser.parse_args()
    if args.overhead_ms <= 0 or any(value <= 0 for value in args.vision_ms):
        parser.error("latencies must be positive")
    try:
        import tiktoken
    except ImportError:
        parser.error("run: uv run --with tiktoken python bench/compare.py")
    if not args.scenarios.exists():
        parser.error("first run: cargo test --release fixture_scan_scenarios -- --ignored --nocapture")
    measured = json.loads(args.scenarios.read_text())
    binary = os.environ.get("SCREENPEEK", str(ROOT / "target/release/screenpeek"))
    fixture = ROOT / "bench/fixtures/dialog.png"
    elements = json.loads(subprocess.check_output([binary, "read", str(fixture), "--json"], text=True))
    target = [e for e in elements if e["text"] == "Save"]
    if len(target) != 1 or not (800 <= target[0]["x"] <= 876 and 480 <= target[0]["y"] <= 514):
        raise SystemExit("Save was not uniquely located inside its button")
    def listing(items):
        return "".join(f'{e["id"]} {e["text"]} @{e["x"]},{e["y"]}\n' for e in items)
    encoding = tiktoken.encoding_for_model("gpt-4.1")
    text_tokens = {"full_listing": len(encoding.encode(listing(elements))),
                   "save_result": len(encoding.encode(listing(target)))}
    width, height = struct.unpack(">II", fixture.read_bytes()[16:24])
    visual = {"full_high": image_tokens(width, height), "full_low": 85,
              "button_row_high": image_tokens(340, 60)}
    estimates = {}
    for name, latency in measured["median_ms"].items():
        total = latency + args.overhead_ms
        estimates[name] = {"measured_recognition_lookup_ms": round(latency, 2),
                           "estimated_local_step_ms": round(total, 2),
                           "assumed_model_ms": {str(ms): {
                               "estimated_speedup": round((ms + args.overhead_ms) / total, 1),
                               "meets_10x": (ms + args.overhead_ms) / total >= 10,
                           } for ms in args.vision_ms}}
    print(json.dumps({
        "task": "locate and click the already-requested Save button",
        "model": "gpt-4.1", "text_encoding": encoding.name,
        "text_payload_tokens": text_tokens, "estimated_image_tokens": visual,
        "payload_reduction_vs_full_high": {name: round(visual["full_high"] / count, 1) for name, count in text_tokens.items()},
        "shared_overhead_ms_assumed": args.overhead_ms, "scenarios": estimates,
        "limits": "Not an end-to-end measurement. Excludes shared prompts, planning, output tokens, application settling and verification. Assumes the screenshot route needs one extra model turn and the semantic route does not. Model-free coordinate clicks and DOM tools are not this baseline.",
        "image_token_source": SOURCE,
    }, indent=2))


if __name__ == "__main__":
    main()
