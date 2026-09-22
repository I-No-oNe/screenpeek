#!/usr/bin/env python3
"""Run scripted desktop tasks with screenpeek and compare observation cost with screenshots.

A task file is a JSON list of {"name": ..., "steps": ["scan --grep Save", "click Save", ...]}.
Each step is a screenpeek command line. Open the apps the tasks need first.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import time

from compare import image_tokens

ROOT = Path(__file__).resolve().parents[1]
OBSERVATIONS = {"scan", "wait", "tree"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tasks", type=Path)
    parser.add_argument("--screen", default="1920x1080", help="screen size a screenshot would have")
    args = parser.parse_args()
    width, height = map(int, args.screen.split("x"))
    screenshot = image_tokens(width, height)
    try:
        import tiktoken
        count = lambda text: len(tiktoken.encoding_for_model("gpt-4.1").encode(text))
    except ImportError:
        count = lambda text: -(-len(text) // 4)  # rough: four characters per token
    binary = os.environ.get("SCREENPEEK", str(ROOT / "target/release/screenpeek"))

    results = []
    for task in json.loads(args.tasks.read_text()):
        started, text_tokens, observations, ok = time.perf_counter(), 0, 0, True
        for step in task["steps"]:
            argv = shlex.split(step)
            done = subprocess.run([binary, *argv], capture_output=True, text=True)
            if argv[0] in OBSERVATIONS:
                observations += 1
                text_tokens += count(done.stdout)
            if done.returncode != 0:
                ok = False
                print(f"{task['name']}: {step!r} failed: {done.stderr.strip()}")
                break
        results.append({
            "task": task["name"],
            "ok": ok,
            "ms": round((time.perf_counter() - started) * 1000),
            "observation_tokens": text_tokens,
            "screenshot_tokens": observations * screenshot,
        })
    print(json.dumps(results, indent=2))
    passed = sum(result["ok"] for result in results)
    print(f"{passed}/{len(results)} tasks passed")


if __name__ == "__main__":
    main()
