#!/usr/bin/env python3
"""Exercise the native title bar and recent popup with isolated, pinned history."""

import argparse
import copy
import ctypes
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

def wait_for(description, probe, predicate, timeout=12):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            last = probe()
        except subprocess.CalledProcessError as error:
            if "(-1719)" not in (error.output or ""):
                raise
            # AX can temporarily omit a window during native full-screen transitions.
            last = error.output
        else:
            if predicate(last):
                print(f"PASS {description}")
                return last
        time.sleep(0.05)
    raise AssertionError(f"{description}: timed out; last observation: {last!r}")


class Point(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


def seed(config):
    repo = config / "navigation-demo"
    subprocess.run(["git", "init", "-b", "main", str(repo)], check=True, capture_output=True)

    def git(*args):
        return subprocess.check_output(["git", "-C", str(repo), *args], text=True, stderr=subprocess.STDOUT).strip()

    git("config", "user.name", "Navigation Test")
    git("config", "user.email", "navigation@example.invalid")

    def commit(text):
        (repo / "file.rs").write_text(text)
        git("add", "file.rs")
        git("commit", "-m", "fixture")
        return git("rev-parse", "HEAD")

    base = commit("before\n")
    git("checkout", "-b", "topic")
    old = commit("pinned reading position\n" * 100)
    new = commit("newer content\n")
    fixture = Path(__file__).resolve().parents[1] / "crates/kestrel/src/viewing_history/storage/fixtures/v1.jsonl"
    header, branches, comparison = map(json.loads, fixture.read_text().splitlines())
    records = []
    for index, head in enumerate([None, old, new]):
        record = copy.deepcopy(comparison if head else branches)
        record.update(id=index, timestamp_ms=1700000000000 + index)
        viewed = record["viewed"]
        viewed["repository"] = {"directory": {"Unix": list(os.fsencode(repo))}, "common_dir": {"Unix": list(os.fsencode(repo / ".git"))}}
        viewed["labels"] = {"repository": repo.name, "directory": str(repo), "branch": "topic" if head else None, "base": "main" if head else None, "file": "file.rs" if head else None}
        view = viewed["snapshot"]["view"]
        view["branches"] = {"query": "", "sort": "Name", "first_visible_row": 0}
        view["browsing"] = {"file": list(b"file.rs") if head else None, "directory": None, "mode": "Diff", "first_visible_line": 0}
        if head:
            branch, base_ref = list(b"refs/heads/topic"), list(b"refs/heads/main")
            viewed["snapshot"]["destination"] = {"Comparison": {"project": 1, "branch": branch, "base": {"Explicit": base_ref}, "comparison": {"branch": branch, "base": base_ref, "base_tip": base, "commits": {"old": base, "new": head}}}}
        records.append(record)
    journal = config / "kestrel/state.history.jsonl"
    journal.parent.mkdir()
    journal.write_text("".join(json.dumps(value) + "\n" for value in [header, *records]))
    return old, journal


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="bring isolated test windows forward")
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/kestrel")
    parser.add_argument("--screenshots", type=Path, help="save real window captures (requires Screen Recording permission)")
    args = parser.parse_args()
    if not args.run:
        parser.print_help()
        return 0
    if sys.platform != "darwin":
        parser.error("macOS is required")
    cg = ctypes.CDLL("/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices")
    cg.AXIsProcessTrusted.restype = ctypes.c_bool
    if not cg.AXIsProcessTrusted():
        parser.error("Accessibility permission is required; no settings were changed")
    cg.CGPreflightScreenCaptureAccess.restype = ctypes.c_bool
    if args.screenshots and not cg.CGPreflightScreenCaptureAccess():
        parser.error("Screen Recording permission is required for screenshots; no settings were changed")
    cg.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, Point, ctypes.c_uint]
    cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    cg.CGEventPost.argtypes = [ctypes.c_uint, ctypes.c_void_p]
    cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
    cf.CFRelease.argtypes = [ctypes.c_void_p]
    with tempfile.TemporaryDirectory(prefix="kestrel-navigation-") as directory:
        config = Path(directory).resolve()
        old, journal = seed(config)
        with (config / "app.log").open("w+") as log:
            app = subprocess.Popen([str(args.binary.resolve(strict=True))], env=dict(os.environ, XDG_CONFIG_HOME=str(config)), stdout=log, stderr=log)
            try:
                def script(body):
                    source = f'tell application "System Events"\ntell (first process whose unix id is {app.pid})\nset frontmost to true\n{body}\nend tell\nend tell'
                    return subprocess.check_output(["osascript", "-e", source], text=True, stderr=subprocess.STDOUT, timeout=15).strip()

                def state():
                    try:
                        return json.loads((config / "kestrel/state.json").read_text())["state"]
                    except (FileNotFoundError, json.JSONDecodeError):
                        return None

                def records():
                    try:
                        return {row["id"]: row for row in map(json.loads, journal.read_text().splitlines()[1:])}
                    except json.JSONDecodeError:
                        return {}

                def count(expected):
                    return wait_for(f"{expected} persisted viewings", records, lambda value: len(value) == expected)

                def mouse(kind, x, y, origin=None):
                    origin = origin or [float(value) for value in script("get position of window 1").split(", ")]
                    event = cg.CGEventCreateMouseEvent(None, kind, Point(origin[0] + x, origin[1] + y), 0)
                    cg.CGEventPost(0, event)
                    cf.CFRelease(event)
                    time.sleep(0.08)

                def click(x, y):
                    mouse(5, x, y)
                    mouse(1, x, y)
                    mouse(2, x, y)
                    time.sleep(0.2)

                def drag(x, y, target_x, target_y):
                    origin = [float(value) for value in script("get position of window 1").split(", ")]
                    for kind, x, y in [(5, x, y), (1, x, y), (6, x + 10, y), (6, target_x, target_y), (2, target_x, target_y)]:
                        mouse(kind, x, y, origin)
                    time.sleep(0.3)

                def popup():
                    width, height = map(float, script("get size of window 1").split(", "))
                    mouse(5, width - 4, height - 4)
                    script('click menu item "Recently Viewed" of menu 1 of menu bar item "Go" of menu bar 1')
                    time.sleep(0.2)

                def capture(name):
                    if not args.screenshots:
                        return
                    source = f'''import AppKit
for row in CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]] {{
    if row[kCGWindowOwnerPID as String] as? Int == {app.pid}, row[kCGWindowLayer as String] as? Int == 0 {{ print(row[kCGWindowNumber as String]!); break }}
}}'''
                    # The Nix C SDK lacks Swift modules; use the system Xcode SDK for captures.
                    environment = {key: value for key, value in os.environ.items() if key not in {"SDKROOT", "DEVELOPER_DIR"}}
                    window_id = subprocess.check_output(["/usr/bin/xcrun", "swift", "-e", source], env=environment, text=True, timeout=30).strip()
                    args.screenshots.mkdir(parents=True, exist_ok=True)
                    subprocess.run(["/usr/sbin/screencapture", "-x", "-l", window_id, str(args.screenshots / f"{name}.png")], check=True, timeout=15)

                wait_for("startup restored one Home tab", state, lambda value: value is not None and len(value["windows"][0]["tabs"]) == 1)
                count(3)
                popup()
                capture("recent-wide")
                script("key code 53")
                time.sleep(0.2)
                count(3)
                popup()
                script("key code 125\nkey code 36")
                viewed = count(4)
                assert viewed[3]["viewed"]["snapshot"]["destination"]["Comparison"]["comparison"]["commits"]["new"] == old
                print("PASS native menu and keyboard reopen older pins, not current refs")
                click(102, 20)
                wait_for("title-bar Back returns Home", state, lambda value: value["windows"][0]["tabs"][0]["route"] == "Home")
                count(4)
                click(132, 20)
                count(5)
                click(162, 20)
                script("key code 36 using {command down}")
                wait_for("clock and Command-Return create a new tab", state, lambda value: len(value["windows"][0]["tabs"]) == 2)
                count(6)
                popup()
                capture("recent-pinned")
                script("key code 53")
                tabs = [tab["id"] for tab in state()["windows"][0]["tabs"]]
                drag(350, 20, 850, 20)
                wait_for("native tab drag reorders after the navigation controls", state, lambda value: [tab["id"] for tab in value["windows"][0]["tabs"]] == tabs[::-1])
                count(7)
                drag(350, 20, 350, 800)
                wait_for("native drag outside the window detaches a tab", state, lambda value: [len(window["tabs"]) for window in value["windows"]] == [1, 1])
                count(8)
                script('keystroke "w" using {command down, shift down}')
                wait_for("close detached window", state, lambda value: len(value["windows"]) == 1)
                script('keystroke "t" using {command down}\nkeystroke "1" using {command down}')
                count(9)
                position = script("get position of window 1")
                drag(80, 20, 120, 60)
                assert script("get position of window 1") != position
                print("PASS window drag region remains usable")
                script("set size of window 1 to {360, 360}")
                time.sleep(0.4)
                popup()
                capture("recent-narrow")
                script("key code 53")
                script('keystroke "f" using {command down, control down}')
                wait_for("full screen enters", lambda: script('get value of attribute "AXFullScreen" of window 1'), lambda value: value == "true")
                time.sleep(0.8)
                popup()
                capture("recent-fullscreen")
                script("key code 53")
                script('keystroke "f" using {command down, control down}')
                wait_for("full screen exits", lambda: script('get value of attribute "AXFullScreen" of window 1'), lambda value: value == "false")
                count(9)
                # AXFullScreen changes before AppKit finishes its exit animation.
                time.sleep(1.)
                script('keystroke "q" using {command down}')
                app.wait(timeout=12)
                assert app.returncode == 0
                print("Native navigation smoke passed; VoiceOver support is not covered.")
            finally:
                if app.poll() is None:
                    app.terminate()
                    app.wait(timeout=5)
                log.seek(0)
                output = log.read()
                if "panicked" in output:
                    raise AssertionError(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
