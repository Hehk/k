#!/usr/bin/env python3
"""Exercise the native title bar and recent popup with isolated, pinned history."""

import argparse
from contextlib import contextmanager
import copy
import ctypes
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import traceback


class SmokeBlocked(Exception):
    pass


class RunArtifacts:
    def __init__(self, parent=None):
        if parent is not None:
            parent.mkdir(parents=True, exist_ok=True)
        # Retain the private directory on success, failure, and interruption.
        self.root = Path(tempfile.mkdtemp(prefix="kestrel-navigation-", dir=parent)).resolve()
        self.started = time.monotonic()
        self.unsupported = []

    def record(self, event, description, **details):
        row = dict(event=event, description=description,
                   elapsed_seconds=round(time.monotonic() - self.started, 3), **details)
        with (self.root / "observations.jsonl").open("a") as log:
            log.write(json.dumps(row) + "\n")

    def check(self, description, condition, observation):
        self.record("passed" if condition else "failed", description, observation=observation)
        if not condition:
            raise AssertionError(description)
        print(f"PASS {description}")

    def unsupported_check(self, description):
        if description not in self.unsupported:
            self.unsupported.append(description)
            self.record("unsupported", description)
            print(f"UNSUPPORTED {description}")

    def snapshot_failure(self):
        # Copy only this run's fixture files, before shutdown can flush new state.
        destination = self.root / "failure-state"
        destination.mkdir(exist_ok=True)
        for relative in ["kestrel/state.json", "kestrel/state.history.jsonl", "app.log"]:
            source = self.root / relative
            if source.is_file() and not (destination / source.name).exists():
                try:
                    shutil.copyfile(source, destination / source.name)
                except OSError as error:
                    self.record("failed", "failure snapshot", file=relative, error=str(error))

    def __enter__(self):
        print(f"Artifacts (retained): {self.root}")
        self.record("started", "native navigation smoke")
        return self

    def __exit__(self, kind, error, tb):
        status = "passed" if error is None else "blocked" if isinstance(error, SmokeBlocked) else "failed"
        if error is not None:
            self.snapshot_failure()
            (self.root / "harness.log").write_text("".join(traceback.format_exception(kind, error, tb)))
        self.record(status, "native navigation smoke", error=str(error) if error else None)
        (self.root / "result.json").write_text(json.dumps({
            "status": status,
            "error": str(error) if error else None,
            "unsupported_checks": self.unsupported,
        }, indent=2) + "\n")
        print(f"Native navigation smoke {status}; artifacts retained at {self.root}")


def wait_for(description, probe, predicate, timeout=12, record=None):
    def observe(event, **details):
        if record:
            record(event, description, **details)

    deadline = time.monotonic() + timeout
    last = None
    observe("waiting", timeout_seconds=timeout)
    try:
        while time.monotonic() < deadline:
            try:
                last = probe()
            except subprocess.CalledProcessError as error:
                if "(-1719)" not in (error.output or ""):
                    raise
                # AX can omit a window during native full-screen transitions.
                last = error.output
                observe("transient", observation=last)
            else:
                observe("observation", observation=last)
                if last is not None and predicate(last):
                    observe("passed", observation=last)
                    print(f"PASS {description}")
                    return last
            time.sleep(0.05)
        raise AssertionError(f"{description}: timed out; last observation: {last!r}")
    except BaseException as error:
        observe("failed", observation=last, error=str(error))
        raise


def stable_for(predicate, seconds=0.5):
    previous, since = None, None

    def stable(value):
        nonlocal previous, since
        now = time.monotonic()
        if not predicate(value):
            since = None
        elif since is None or value != previous:
            since = now
        previous = value
        return since is not None and now - since >= seconds

    return stable


@contextmanager
def application(command, evidence):
    with (evidence.root / "app.log").open("w+", encoding="utf-8", errors="replace") as log:
        app = subprocess.Popen(command, env=dict(os.environ, XDG_CONFIG_HOME=str(evidence.root)),
                               stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        evidence.record("started", "isolated application", pid=app.pid, command=command)
        failed = False
        try:
            yield app
        except BaseException:
            failed = True
            evidence.snapshot_failure()
            raise
        finally:
            try:
                if app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        evidence.record("cleanup", "terminate timed out; killing isolated application")
                        app.kill()
                        app.wait(timeout=5)
                evidence.record("exited", "isolated application", returncode=app.returncode)
            except Exception as error:
                evidence.record("failed", "application cleanup", error=str(error))
                if not failed:
                    raise
            log.seek(0)
            if "panicked" in log.read():
                evidence.record("failed", "application log contains panic text")
                if not failed:
                    raise AssertionError("application panicked; see app.log")


class Point(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


def seed(config):
    repo = config / "navigation-demo"
    repo.mkdir()
    # Match watch_refs.rs: inherit only PATH, never Git overrides or user config.
    environment = {
        "PATH": os.environ.get("PATH", os.defpath),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_AUTHOR_NAME": "Navigation Test",
        "GIT_AUTHOR_EMAIL": "navigation@example.invalid",
        "GIT_COMMITTER_NAME": "Navigation Test",
        "GIT_COMMITTER_EMAIL": "navigation@example.invalid",
    }

    def git(*args):
        return subprocess.check_output(
            ["git", "-C", str(repo), "-c", f"core.hooksPath={os.devnull}",
             "-c", "commit.gpgsign=false", *args],
            env=environment,
            stdin=subprocess.DEVNULL,
            text=True,
            stderr=subprocess.STDOUT,
            timeout=15,
        ).strip()

    git("init", "--template=", "-b", "main")

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
    parser.add_argument("--artifacts", type=Path, help="parent for a private retained run directory (default: system temp directory)")
    parser.add_argument("--screenshots", type=Path, help="parent for private window captures (requires Screen Recording permission)")
    args = parser.parse_args()
    if not args.run:
        parser.print_help()
        return 0
    try:
        with RunArtifacts(args.artifacts) as evidence:
            run_native(args, evidence)
    except SmokeBlocked as error:
        print(f"BLOCKED {error}", file=sys.stderr)
        return 2
    except Exception as error:
        print(f"FAILED {error}", file=sys.stderr)
        return 1
    return 0


def run_native(args, evidence):
    if sys.platform != "darwin":
        raise SmokeBlocked("macOS is required; no app was launched")
    cg = ctypes.CDLL("/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices")
    cg.AXIsProcessTrusted.restype = ctypes.c_bool
    if not cg.AXIsProcessTrusted():
        raise SmokeBlocked("Accessibility permission is required; no settings were changed or app launched")
    cg.CGPreflightScreenCaptureAccess.restype = ctypes.c_bool
    if args.screenshots and not cg.CGPreflightScreenCaptureAccess():
        raise SmokeBlocked("Screen Recording permission is required for screenshots; no settings were changed or app launched")
    cg.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, Point, ctypes.c_uint]
    cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    cg.CGEventPost.argtypes = [ctypes.c_uint, ctypes.c_void_p]
    cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
    cf.CFRelease.argtypes = [ctypes.c_void_p]
    evidence.unsupported_check("Custom popup visibility, focus, and paint readiness have no AX/state probe; popup delays are input pacing, not verification.")
    evidence.unsupported_check("Custom title-bar hit targets have no AX bounds; coordinates assume the default layout. Persisted effects, not hit targets, are checked.")
    evidence.unsupported_check("Stable AX geometry does not prove AppKit animation or rendering has completed.")
    evidence.unsupported_check("VoiceOver support is not covered.")
    config = evidence.root
    evidence.record("started", "seed isolated Git and history fixture")
    old, journal = seed(config)
    evidence.record("passed", "seed isolated Git and history fixture")
    with application([str(args.binary.resolve(strict=True))], evidence) as app:
        def script(body):
            evidence.record("action", "AppleScript", source=body)
            source = f'tell application "System Events"\ntell (first process whose unix id is {app.pid})\nset frontmost to true\n{body}\nend tell\nend tell'
            try:
                output = subprocess.check_output(["osascript", "-e", source], text=True, stderr=subprocess.STDOUT, timeout=15).strip()
            except subprocess.CalledProcessError as error:
                evidence.record("error", "AppleScript", output=error.output)
                raise
            evidence.record("observation", "AppleScript", output=output)
            return output

        def wait(description, probe, predicate, timeout=12):
            def checked_probe():
                if app.poll() is not None:
                    raise AssertionError(f"application exited early ({app.returncode}); see app.log")
                return probe()
            return wait_for(description, checked_probe, predicate, timeout, evidence.record)

        def geometry():
            return list(map(float, script("get {position of window 1, size of window 1}").split(", ")))

        def ready_geometry():
            return wait("native window geometry available", geometry,
                        lambda value: len(value) == 4 and value[2] > 0 and value[3] > 0)

        def popup_pacing():
            evidence.record("unsupported", "popup readiness unobservable; pacing input", seconds=0.2)
            time.sleep(0.2)

        def state():
            try:
                return json.loads((config / "kestrel/state.json").read_text())["state"]
            except (FileNotFoundError, json.JSONDecodeError):
                return None

        def records():
            try:
                return {row["id"]: row for row in map(json.loads, journal.read_text().splitlines()[1:])}
            except (FileNotFoundError, json.JSONDecodeError):
                return None

        def count(expected):
            return wait(f"{expected} persisted viewings", records, lambda value: len(value) == expected)

        def mouse(kind, x, y, origin=None):
            origin = origin or ready_geometry()[:2]
            evidence.record("action", "mouse event", kind=kind, offset=[x, y], origin=origin)
            event = cg.CGEventCreateMouseEvent(None, kind, Point(origin[0] + x, origin[1] + y), 0)
            cg.CGEventPost(0, event)
            cf.CFRelease(event)
            # Pace the injected event sequence; effects are awaited separately.
            time.sleep(0.08)

        def click(x, y):
            origin = ready_geometry()[:2]
            mouse(5, x, y, origin)
            mouse(1, x, y, origin)
            mouse(2, x, y, origin)

        def drag(x, y, target_x, target_y):
            origin = ready_geometry()[:2]
            for kind, x, y in [(5, x, y), (1, x, y), (6, x + 10, y), (6, target_x, target_y), (2, target_x, target_y)]:
                mouse(kind, x, y, origin)

        def popup():
            width, height = ready_geometry()[2:]
            mouse(5, width - 4, height - 4)
            wait("Recently Viewed native menu available",
                 lambda: script('exists menu item "Recently Viewed" of menu 1 of menu bar item "Go" of menu bar 1'),
                 lambda value: value == "true")
            script('click menu item "Recently Viewed" of menu 1 of menu bar item "Go" of menu bar 1')
            popup_pacing()

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
            if not window_id.isdecimal():
                raise AssertionError("no isolated application window found for capture")
            subprocess.run(["/usr/sbin/screencapture", "-x", "-l", window_id, str(screenshots / f"{name}.png")], check=True, timeout=15)
            evidence.record("observation", "window capture (not visual acceptance)", path=str(screenshots / f"{name}.png"))

        screenshots = None
        if args.screenshots:
            args.screenshots.mkdir(parents=True, exist_ok=True)
            screenshots = Path(tempfile.mkdtemp(prefix="kestrel-navigation-", dir=args.screenshots)).resolve()
            evidence.record("artifact", "private window captures", path=str(screenshots))
        wait("startup restored one Home tab", state, lambda value: len(value["windows"]) == 1 and len(value["windows"][0]["tabs"]) == 1 and value["windows"][0]["tabs"][0]["route"] == "Home")
        count(3)
        popup()
        capture("recent-wide")
        script("key code 53")
        popup_pacing()
        count(3)
        popup()
        script("key code 125\nkey code 36")
        viewed = count(4)
        restored = viewed[3]["viewed"]["snapshot"]["destination"]["Comparison"]["comparison"]["commits"]["new"]
        evidence.check("native menu and keyboard reopen older pins, not current refs", restored == old, restored)
        click(102, 20)
        wait("title-bar Back returns Home", state, lambda value: value["windows"][0]["tabs"][0]["route"] == "Home")
        count(4)
        click(132, 20)
        count(5)
        click(162, 20)
        popup_pacing()
        script("key code 36 using {command down}")
        opened = wait("clock and Command-Return create a new tab", state, lambda value: len(value["windows"][0]["tabs"]) == 2)
        selected = opened["windows"][0]["active_tab"]
        tab = next(tab for tab in opened["windows"][0]["tabs"] if tab["id"] == selected)
        evidence.check("saved-view tab starts without a Home history entry", tab["back"] == [] and tab["forward"] == [], tab)
        count(6)
        popup()
        capture("recent-pinned")
        script("key code 53")
        tabs = [tab["id"] for tab in opened["windows"][0]["tabs"]]
        drag(350, 20, 850, 20)
        wait("native tab drag reorders after the navigation controls", state, lambda value: [tab["id"] for tab in value["windows"][0]["tabs"]] == tabs[::-1])
        count(7)
        drag(350, 20, 350, 800)
        wait("native drag outside the window detaches a tab", state, lambda value: [len(window["tabs"]) for window in value["windows"]] == [1, 1])
        count(8)
        script('keystroke "w" using {command down, shift down}')
        wait("close detached window", state, lambda value: len(value["windows"]) == 1)
        script('keystroke "t" using {command down}\nkeystroke "1" using {command down}')
        count(9)
        position = ready_geometry()[:2]
        drag(80, 20, 120, 60)
        wait("window drag region remains usable", geometry, lambda value: value[:2] != position)
        script("set size of window 1 to {360, 360}")
        wait("narrow window geometry settles", geometry, stable_for(lambda value: value[2:] == [360, 360]))
        popup()
        capture("recent-narrow")
        script("key code 53")
        script('keystroke "f" using {command down, control down}')

        def fullscreen_geometry():
            return [script('get value of attribute "AXFullScreen" of window 1'), geometry()]

        wait("full screen enters and geometry settles", fullscreen_geometry, stable_for(lambda value: value[0] == "true"))
        popup()
        capture("recent-fullscreen")
        script("key code 53")
        script('keystroke "f" using {command down, control down}')
        wait("full screen exits and geometry settles", fullscreen_geometry, stable_for(lambda value: value[0] == "false"))
        count(9)
        script('keystroke "q" using {command down}')
        app.wait(timeout=12)
        evidence.check("application quits successfully", app.returncode == 0, app.returncode)


if __name__ == "__main__":
    raise SystemExit(main())
