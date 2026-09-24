#!/usr/bin/env python3
"""Measure the release CLI; assert accuracy and optional local latency budgets."""
import argparse
import json
import os
from pathlib import Path
import statistics
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def run(command, env=None):
    started = time.perf_counter()
    result = subprocess.run(command, env=env, check=True, capture_output=True, text=True, timeout=120)
    return (time.perf_counter() - started) * 1000, result.stdout


def measure(command, runs, env=None):
    samples = [run(command, env) for _ in range(runs)]
    times = sorted(t for t, _ in samples)
    return {"median_ms": round(statistics.median(times), 1), "min_ms": round(times[0], 1),
            "max_ms": round(times[-1], 1), "runs": runs}, samples


def resolve(label, elements):
    """Resolve exact, prefix or substring matches; ambiguity is a failure."""
    needle = label.lower()
    for matches in (lambda t: t == needle, lambda t: t.startswith(needle), lambda t: needle in t):
        hits = [e for e in elements if matches(e["text"].lower())]
        if len(hits) == 1:
            return hits[0]
        if hits:
            return None
    return None


def clickable(label, elements, targets):
    """Reachable by name, and the click lands on the right control."""
    hit = resolve(label, elements)
    if hit is None:
        return False
    target = next((t for t in targets if t["label"] == label), None)
    if target is None:
        return True
    return (target["x"] <= hit["x"] < target["x"] + target["width"]
            and target["y"] <= hit["y"] < target["y"] + target["height"])


# Arabic baseline is 7/12; raise its floor after recognition improves.
FLOORS = {}
DEFAULT_FLOOR = 0.90


def floor(name):
    return FLOORS.get(name, DEFAULT_FLOOR)


def measure_fixture(binary, fixture, language, scale, runs):
    command = [binary, "read", str(fixture), "--json", "--scale", str(scale)]
    if language:
        command += ["--lang", language]
    run(command)  # Exclude first-use model download and filesystem warmup.
    stats, samples = measure(command, runs)
    expected = set(fixture.with_suffix(".expected").read_text().splitlines())
    width, height = struct.unpack(">II", fixture.read_bytes()[16:24])
    targets_path = fixture.with_suffix(".json")
    targets = json.loads(targets_path.read_text())["targets"] if targets_path.exists() else []
    recalls = []
    resolvables = []
    missing_all = set()
    unresolvable_all = set()
    for _, output in samples:
        elements = json.loads(output)
        found = {e["text"] for e in elements}
        recall = len(expected & found) / len(expected)
        reachable = {label for label in expected if clickable(label, elements, targets)}
        resolvables.append(len(reachable) / len(expected))
        missing_all.update(expected - found)
        unresolvable_all.update(expected - reachable)
        if not all(e["width"] > 0 and e["height"] > 0 and 0 <= e["x"] < width and 0 <= e["y"] < height for e in elements):
            raise SystemExit("invalid fixture coordinates")
        for target in targets:
            hits = [e for e in elements if e["text"] == target["label"]]
            if len(hits) == 1 and not (target["x"] <= hits[0]["x"] < target["x"] + target["width"]
                    and target["y"] <= hits[0]["y"] < target["y"] + target["height"]):
                raise SystemExit(f"wrong click point for {target['label']!r}")
        recalls.append(recall)
    stats.update(recall=min(recalls), resolvable=min(resolvables), missing=sorted(missing_all),
                 unresolvable=sorted(unresolvable_all), bytes=len(samples[-1][1].encode()),
                 language=language, scale=scale)
    return stats


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["ocr", "screen"], nargs="?", default="ocr")
    parser.add_argument("--all", action="store_true", help="gate every fixture, using its language model")
    parser.add_argument("--scale", type=int, choices=range(1, 5), default=1)
    parser.add_argument("--fixture", type=Path, default=ROOT / "bench/fixtures/dialog.png")
    parser.add_argument("--lang", help="Tesseract language code, e.g. eng, heb, jpn")
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--max-ms", type=float, help="fail when a median exceeds this local budget")
    parser.add_argument("--require-languages", action="store_true", help="with --all, fail instead of skipping fixtures whose language is not installed")
    args = parser.parse_args()
    if args.runs < 3:
        parser.error("use at least 3 measured runs")
    binary = str(Path(os.environ.get("SCREENPEEK", ROOT / "target/release/screenpeek")).resolve())
    if args.mode == "ocr":
        fixtures = sorted((ROOT / "bench/fixtures").glob("*.json")) if args.all else [args.fixture]
        report = {}
        installed = set(run([binary, "languages"])[1].split()) if args.all else set()
        for fixture in fixtures:
            language = args.lang
            if args.all:
                manifest = json.loads(fixture.read_text())
                language = manifest.get("language") if fixture.stem != "dialog" else None
                missing = set((language or "").split("+")) - installed - {""}
                if missing and not args.require_languages:
                    print(f"skipping {fixture.stem}: {' '.join(sorted(missing))} not installed", file=sys.stderr)
                    continue
                fixture = fixture.parent / manifest["image"]
            report[fixture.stem] = measure_fixture(binary, fixture, language, args.scale, args.runs)
    else:
        # Keep model files, endpoints and snapshots isolated from the user's daemon.
        with tempfile.TemporaryDirectory(prefix="screenpeek-bench-") as work:
            env = dict(os.environ, XDG_CACHE_HOME=work)
            cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "screenpeek"
            local = Path(work) / "screenpeek"
            local.mkdir()
            for model in cache.glob("*.rten"):
                (local / model.name).symlink_to(model.resolve())
            cold_env = dict(env, SCREENPEEK_NO_DAEMON="1")
            command = [binary, "scan", "--json"]
            if args.lang:
                command += ["--lang", args.lang]
            run(command, cold_env)
            cold, _ = measure(command, args.runs, cold_env)
            env.pop("SCREENPEEK_NO_DAEMON", None)
            with open(Path(work) / "daemon.log", "w+") as log:
                daemon = subprocess.Popen([binary, "serve"], env=env, stdout=subprocess.DEVNULL, stderr=log)
                try:
                    deadline = time.monotonic() + 30
                    while not (local / "daemon-v3").exists():
                        if daemon.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError("benchmark daemon did not start")
                        time.sleep(.05)
                    run(command, env)
                    warm, samples = measure(command, args.runs, env)
                    log.flush()
                    log.seek(0)
                    modes = log.read().splitlines()
                    if not any(line.startswith(("full:", "unchanged:", "patched:", "window only:")) for line in modes):
                        raise RuntimeError("scan did not use the benchmark daemon")
                    warm["elements"] = len(json.loads(samples[-1][1]))
                    report = {"no_daemon": cold, "daemon": warm, "daemon_log": modes}
                finally:
                    daemon.terminate()
                    try:
                        daemon.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        daemon.kill()
                        daemon.wait()
    print(json.dumps(report, indent=2))
    failed = [
        f"{name}: {stats['resolvable']:.2f} resolvable, floor {floor(name):.2f}, "
        f"cannot reach {stats['unresolvable']}"
        for name, stats in report.items()
        if isinstance(stats, dict) and stats.get("resolvable", 1) < floor(name)
    ]
    if failed:
        raise SystemExit("below the floor:\n  " + "\n  ".join(failed))
    if args.max_ms is not None and any(isinstance(stats, dict) and stats["median_ms"] > args.max_ms for stats in report.values()):
        raise SystemExit(f"median exceeded {args.max_ms} ms")


if __name__ == "__main__":
    main()
