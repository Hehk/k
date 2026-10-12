#!/usr/bin/env python3
"""Exercise native shortcuts in an isolated Kestrel process (requires Accessibility)."""

import argparse
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
            time.sleep(0.05)
            continue
        if predicate(last):
            print(f"PASS {description}")
            return last
        time.sleep(0.05)
    raise AssertionError(f"{description}: timed out; last observation: {last!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="bring test windows forward and send keystrokes")
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/kestrel")
    args = parser.parse_args()
    if not args.run:
        parser.print_help()
        return 0
    if sys.platform != "darwin":
        parser.error("macOS is required")
    ax = ctypes.CDLL("/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices")
    ax.AXIsProcessTrusted.restype = ctypes.c_bool
    if not ax.AXIsProcessTrusted():
        parser.error("grant Accessibility to the launching terminal, then retry; no settings were changed")
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="kestrel-shortcuts-") as directory:
        config = Path(directory)
        state_path = config / "kestrel/state.json"
        env = dict(os.environ, XDG_CONFIG_HOME=str(config))
        with (config / "app.log").open("w+") as log:
            app = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
            try:
                def state():
                    if app.poll() is not None:
                        raise RuntimeError(f"Kestrel exited with {app.returncode}")
                    try:
                        return json.loads(state_path.read_text())["state"]
                    except (FileNotFoundError, json.JSONDecodeError):
                        return None

                def script(body, activate=True):
                    source = f'''tell application "System Events"
                        tell (first process whose unix id is {app.pid})
                            {"set frontmost to true" if activate else ""}
                            {body}
                        end tell
                    end tell'''
                    return subprocess.check_output(["/usr/bin/osascript", "-e", source], text=True, stderr=subprocess.STDOUT, timeout=15).strip()

                def key(character, *modifiers):
                    script(f'keystroke {json.dumps(character)} using {{{", ".join(modifiers)}}}')

                def menu(label, group="File"):
                    script(f'click menu item {json.dumps(label, ensure_ascii=False)} of menu 1 of menu bar item {json.dumps(group)} of menu bar 1')

                def expect(description, counts):
                    return wait_for(description, state, lambda value: value is not None and [len(w["tabs"]) for w in value["windows"]] == counts)

                expect("startup: one Home tab", [1])
                for label, group in [("Reopen Closed Tab or Window", "File"), ("Move Tab to New Window", "Window"), ("Close Other Tabs", "Window")]:
                    enabled = script(f'get enabled of menu item {json.dumps(label)} of menu 1 of menu bar item {json.dumps(group)} of menu bar 1')
                    assert enabled == "false", f"{label} should be disabled: {enabled}"
                print("PASS native menu enablement with one tab and empty history")
                original_size = script('get value of attribute "AXSize" of window 1')
                key("f", "command down", "control down")
                wait_for("native full screen enters", lambda: script('get value of attribute "AXFullScreen" of window 1'), lambda value: value == "true")
                wait_for("native full screen menu changes title", lambda: script('get exists menu item "Exit Full Screen" of menu 1 of menu bar item "Window" of menu bar 1'), lambda value: value == "true")
                menu("Exit Full Screen", "Window")
                wait_for("native full screen exits", lambda: script('get value of attribute "AXFullScreen" of window 1'), lambda value: value == "false")
                wait_for("full screen restores window size", lambda: script('get value of attribute "AXSize" of window 1'), lambda value: value == original_size)
                key("m", "command down")
                wait_for("native Command-M minimizes", lambda: script('get value of attribute "AXMinimized" of window 1', activate=False), lambda value: value == "true")
                menu("Home — Kestrel", "Window")
                wait_for("native Window list restores minimized window", lambda: script('get value of attribute "AXMinimized" of window 1'), lambda value: value == "false")
                key("h", "command down")
                wait_for("native Command-H hides only Kestrel", lambda: script('get visible', activate=False), lambda value: value == "false")
                wait_for("activation restores hidden Kestrel", lambda: script('get visible'), lambda value: value == "true")
                key("t", "command down")
                expect("Command-T adds exactly one tab", [2])
                for _ in range(8):
                    key("t", "command down")
                expect("ten tabs", [10])
                key("1", "command down")
                wait_for("Command-1 selects first", state, lambda value: value["windows"][0]["active_tab"] == value["windows"][0]["tabs"][0]["id"])
                key("9", "command down")
                wait_for("Command-9 selects last, not ninth", state, lambda value: value["windows"][0]["active_tab"] == value["windows"][0]["tabs"][-1]["id"])
                script("key code 48 using {control down}")
                wait_for("Control-Tab wraps to first", state, lambda value: value["windows"][0]["active_tab"] == value["windows"][0]["tabs"][0]["id"])
                script("key code 48 using {control down, shift down}")
                wait_for("Control-Shift-Tab wraps to last", state, lambda value: value["windows"][0]["active_tab"] == value["windows"][0]["tabs"][-1]["id"])
                key("]", "command down", "shift down")
                wait_for("Command-Shift-bracket wraps to first", state, lambda value: value["windows"][0]["active_tab"] == value["windows"][0]["tabs"][0]["id"])
                key("[", "command down", "shift down")
                wait_for("Command-Shift-bracket wraps to last", state, lambda value: value["windows"][0]["active_tab"] == value["windows"][0]["tabs"][-1]["id"])
                key("w", "command down")
                expect("Command-W closes only one tab", [9])
                key("t", "command down", "shift down")
                expect("Command-Shift-T reopens a tab", [10])
                key("n", "command down")
                expect("Command-N opens an independent window", [10, 1])
                key("t", "command down")
                windows = expect("tab creation targets front window", [10, 2])["windows"]
                key("`", "command down")
                wait_for("native Command-backtick cycles windows, not tabs", state, lambda value: value["active_window"] == windows[0]["id"])
                key("`", "command down", "shift down")
                wait_for("native reverse window cycling", state, lambda value: value["active_window"] == windows[1]["id"])
                key("w", "command down", "shift down")
                expect("Command-Shift-W closes whole front window", [10])
                key("t", "command down", "shift down")
                expect("Command-Shift-T restores a closed window", [10, 2])
                menu("Close Other Tabs", "Window")
                expect("native menu closes other tabs", [10, 1])
                key("w", "command down", "option down")
                expect("Command-Option-W closes all windows, not app", [])
                enabled = script('get enabled of menu item "Close Window" of menu 1 of menu bar item "File" of menu bar 1')
                assert enabled == "false", f"Close Window should be disabled: {enabled}"
                key("n", "command down")
                expect("native Command-N works with zero windows", [1])
                key("w", "command down")
                expect("last-tab close leaves app running", [])
                key("t", "command down")
                expect("native Command-T works with zero windows", [1])
                key("t", "command down")
                before = expect("prepare quit restoration", [2])
                key("q", "command down")
                app.wait(timeout=12)
                assert app.returncode == 0, f"Quit returned {app.returncode}"
                assert json.loads(state_path.read_text())["state"] == before
                print("PASS native Command-Q preserves session")
                app = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
                expect("relaunch restores the saved window", [2])
                key("q", "command down")
                app.wait(timeout=12)
                print("Native smoke passed. Full acceptance still requires non-US layouts, Hide Others/Show All, Dock/Spaces behavior, menu overrides, and accessibility review.")
            except Exception:
                log.flush()
                log.seek(0)
                print(log.read()[-4000:], file=sys.stderr)
                raise
            finally:
                if app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        app.kill()
                        app.wait()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
