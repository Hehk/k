"""Headless smoke-harness regressions; no native application is launched."""

import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import macos_navigation_smoke as smoke
from macos_navigation_smoke import seed


class SeedTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="kestrel-smoke-git-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.home = self.root / "home"
        self.home.mkdir()
        self.environment = {
            "PATH": os.environ.get("PATH", os.defpath),
            "HOME": str(self.home),
            "XDG_CONFIG_HOME": str(self.home / ".config"),
            "GIT_CONFIG_NOSYSTEM": "1",
        }

    def git(self, repo, *args):
        return subprocess.check_output(
            ["git", "-C", str(repo), *args],
            env={
                "PATH": self.environment["PATH"],
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": os.devnull,
            },
            stdin=subprocess.DEVNULL,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=15,
        ).strip()

    def snapshot(self, directory):
        return {
            str(path.relative_to(directory)): (path.stat().st_mode, path.read_bytes())
            for path in directory.rglob("*")
            if path.is_file()
        }

    def assert_seeded(self, config):
        old, journal = seed(config)
        repo = config / "navigation-demo"
        self.assertEqual(self.git(repo, "rev-parse", "--show-toplevel"), str(repo))
        base = self.git(repo, "rev-parse", "main")
        new = self.git(repo, "rev-parse", "topic")
        self.assertEqual(self.git(repo, "rev-list", "--count", "topic"), "3")
        self.assertEqual(self.git(repo, "rev-parse", "topic^"), old)
        self.assertEqual(self.git(repo, "rev-parse", f"{old}^"), base)
        self.assertEqual(self.git(repo, "show", f"{old}:file.rs"), ("pinned reading position\n" * 100).strip())
        self.assertEqual((repo / "file.rs").read_text(), "newer content\n")
        self.assertEqual(self.git(repo, "status", "--porcelain"), "")
        self.assertEqual(self.git(repo, "show", "-s", "--format=%an <%ae>|%cn <%ce>", "topic"),
                         "Navigation Test <navigation@example.invalid>|Navigation Test <navigation@example.invalid>")
        _, _, pinned, latest = map(json.loads, journal.read_text().splitlines())
        for record, head in [(pinned, old), (latest, new)]:
            comparison = record["viewed"]["snapshot"]["destination"]["Comparison"]["comparison"]
            self.assertEqual(comparison["commits"], {"old": base, "new": head})
            self.assertEqual(comparison["base_tip"], base)
        self.assertEqual(list((repo / ".git" / "hooks").glob("*")), [])

    def test_seed_ignores_hostile_user_and_system_configuration(self):
        hooks = self.home / "hooks"
        hooks.mkdir()
        # Executable failures make accidental hooks/signing fail without invoking credentials.
        (hooks / "pre-commit").symlink_to("/usr/bin/false")
        template = self.home / "template"
        template.mkdir()
        (template / "copied-from-user-template").write_text("must not be copied\n")
        hostile_config = (
            f'[core]\n hooksPath = "{hooks}"\n'
            '[commit]\n gpgSign = true\n'
            '[gpg]\n program = /usr/bin/false\n'
            f'[init]\n templateDir = "{template}"\n'
            '[user]\n name = Hostile Identity\n email = hostile@example.invalid\n'
        )
        global_config = self.home / ".gitconfig"
        global_config.write_text(hostile_config)
        xdg_config = self.home / ".config" / "git" / "config"
        xdg_config.parent.mkdir(parents=True)
        xdg_config.write_text(hostile_config)
        system_config = self.home / "system.gitconfig"
        system_config.write_text(hostile_config)
        before = self.snapshot(self.home)
        environments = [
            self.environment,
            dict(self.environment, GIT_CONFIG_GLOBAL=str(global_config),
                 GIT_CONFIG_SYSTEM=str(system_config), GIT_CONFIG_NOSYSTEM="0"),
        ]
        for index, environment in enumerate(environments):
            with self.subTest(environment=index):
                config = self.root / f"fixture-{index}"
                config.mkdir()
                with patch.dict(os.environ, environment, clear=True):
                    self.assert_seeded(config)
                    self.assertEqual(dict(os.environ), environment)
                self.assertFalse((config / "navigation-demo/.git/copied-from-user-template").exists())
                self.assertEqual(self.snapshot(self.home), before)

    def test_seed_cannot_be_redirected_to_an_unrelated_repository(self):
        unrelated = self.root / "unrelated"
        unrelated.mkdir()
        self.git(unrelated, "init", "--template=", "-b", "main")
        (unrelated / "file.rs").write_text("unrelated staged content\n")
        self.git(unrelated, "add", "file.rs")
        self.git(unrelated, "-c", "user.name=Unrelated", "-c", "user.email=unrelated@example.invalid",
                 "-c", "commit.gpgsign=false", "-c", f"core.hooksPath={os.devnull}",
                 "commit", "-m", "unrelated")
        (unrelated / "file.rs").write_text("unrelated dirty content\n")
        before = self.snapshot(unrelated)
        redirected = dict(self.environment, GIT_DIR=str(unrelated / ".git"), GIT_WORK_TREE=str(unrelated))
        all_overrides = dict(
            redirected,
            GIT_COMMON_DIR=str(unrelated / ".git"),
            GIT_INDEX_FILE=str(unrelated / ".git/index"),
            GIT_OBJECT_DIRECTORY=str(unrelated / ".git/objects"),
            GIT_TEMPLATE_DIR=str(unrelated / ".git"),
            GIT_CONFIG_COUNT="1",
            GIT_CONFIG_KEY_0="core.bare",
            GIT_CONFIG_VALUE_0="true",
            GIT_AUTHOR_NAME="Wrong Author",
            GIT_COMMITTER_EMAIL="wrong@example.invalid",
        )
        for index, environment in enumerate([redirected, all_overrides]):
            with self.subTest(environment=index):
                config = self.root / f"fixture-{index}"
                config.mkdir()
                with patch.dict(os.environ, environment, clear=True):
                    self.assert_seeded(config)
                    self.assertEqual(dict(os.environ), environment)
                self.assertEqual(self.snapshot(unrelated), before)


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="kestrel-smoke-evidence-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.stdout = patch("sys.stdout", new_callable=io.StringIO).start()
        self.addCleanup(patch.stopall)

    def observations(self, evidence):
        return list(map(json.loads, (evidence.root / "observations.jsonl").read_text().splitlines()))

    def result(self, evidence):
        return json.loads((evidence.root / "result.json").read_text())

    def test_success_retains_private_evidence_and_unsupported_checks(self):
        with smoke.RunArtifacts(self.root) as evidence:
            seed(evidence.root)
            evidence.unsupported_check("popup paint is not observable")
            smoke.wait_for("persisted", lambda: {"count": 3}, lambda value: value["count"] == 3,
                           record=evidence.record)
        self.assertEqual(self.result(evidence)["status"], "passed")
        self.assertEqual(self.result(evidence)["unsupported_checks"], ["popup paint is not observable"])
        self.assertTrue((evidence.root / "kestrel/state.history.jsonl").is_file())
        self.assertEqual(evidence.root.stat().st_mode & 0o777, 0o700)
        self.assertIn("UNSUPPORTED", self.stdout.getvalue())
        self.assertIn(str(evidence.root), self.stdout.getvalue())
        self.assertTrue(any(row["event"] == "observation" and row["observation"] == {"count": 3}
                            for row in self.observations(evidence)))

    def test_failed_check_records_the_actual_value(self):
        with self.assertRaisesRegex(AssertionError, "older pins"):
            with smoke.RunArtifacts(self.root) as evidence:
                evidence.check("older pins", False, {"expected": "old", "actual": "new"})
        self.assertEqual(self.result(evidence)["status"], "failed")
        self.assertTrue(any(row["event"] == "failed" and row.get("observation") == {"expected": "old", "actual": "new"}
                            for row in self.observations(evidence)))

    def test_timeout_retains_last_observation_and_original_files(self):
        with self.assertRaisesRegex(AssertionError, "timed out; last observation: 3"):
            with smoke.RunArtifacts(self.root) as evidence:
                state = evidence.root / "kestrel/state.json"
                state.parent.mkdir()
                state.write_bytes(b'{"interrupted":')
                (state.parent / "state.history.jsonl").write_bytes(b'fixture journal\n')
                (evidence.root / "app.log").write_text("non-panic diagnostic\n")
                smoke.wait_for("four viewings", lambda: 3, lambda value: value == 4,
                               timeout=0.01, record=evidence.record)
        self.assertEqual(self.result(evidence)["status"], "failed")
        self.assertEqual((evidence.root / "failure-state/state.json").read_bytes(), b'{"interrupted":')
        self.assertEqual((evidence.root / "failure-state/state.history.jsonl").read_bytes(), b'fixture journal\n')
        self.assertEqual((evidence.root / "failure-state/app.log").read_text(), "non-panic diagnostic\n")
        self.assertIn("four viewings", (evidence.root / "harness.log").read_text())
        self.assertTrue(any(row["event"] == "failed" and row.get("observation") == 3
                            for row in self.observations(evidence)))

    def test_assertions_and_interruptions_are_not_reported_as_passes(self):
        for exception in [AssertionError("wrong pins"), KeyboardInterrupt()]:
            with self.subTest(exception=type(exception).__name__):
                with self.assertRaises(type(exception)):
                    with smoke.RunArtifacts(self.root) as evidence:
                        evidence.record("action", "reopen pinned view")
                        raise exception
                self.assertEqual(self.result(evidence)["status"], "failed")
                self.assertTrue((evidence.root / "harness.log").is_file())

    def test_failure_snapshot_precedes_shutdown_and_cleanup_does_not_mask_failure(self):
        app = Mock(pid=123, returncode=-15)
        app.poll.return_value = None
        with self.assertRaisesRegex(AssertionError, "original failure"):
            with smoke.RunArtifacts(self.root) as evidence:
                state = evidence.root / "kestrel/state.json"
                state.parent.mkdir()
                state.write_text("before cleanup")

                def terminate():
                    state.write_text("shutdown flushed state")
                    raise OSError("cleanup failure")

                app.terminate.side_effect = terminate
                with patch.object(smoke.subprocess, "Popen", return_value=app):
                    with smoke.application(["never-launched"], evidence):
                        raise AssertionError("original failure")
        self.assertEqual((evidence.root / "failure-state/state.json").read_text(), "before cleanup")
        self.assertEqual(state.read_text(), "shutdown flushed state")
        self.assertIn("original failure", self.result(evidence)["error"])
        self.assertTrue(any(row["description"] == "application cleanup" and row["error"] == "cleanup failure"
                            for row in self.observations(evidence)))

    def test_headless_child_log_and_config_are_isolated_on_success(self):
        unrelated = self.root / "real-config"
        unrelated.mkdir()
        sentinel = unrelated / "state.json"
        sentinel.write_text("private user state must not be read or copied")
        with patch.dict(os.environ, XDG_CONFIG_HOME=str(unrelated)):
            with smoke.RunArtifacts(self.root) as evidence:
                code = "import os; from pathlib import Path; p = Path(os.environ['XDG_CONFIG_HOME']); (p / 'child-marker').write_text('fixture'); print('ordinary diagnostic')"
                with smoke.application([sys.executable, "-c", code], evidence) as app:
                    self.assertEqual(app.wait(timeout=5), 0)
        self.assertEqual(self.result(evidence)["status"], "passed")
        self.assertEqual((evidence.root / "child-marker").read_text(), "fixture")
        self.assertEqual((evidence.root / "app.log").read_text(), "ordinary diagnostic\n")
        self.assertEqual(sentinel.read_text(), "private user state must not be read or copied")
        self.assertFalse((unrelated / "child-marker").exists())
        for path in evidence.root.rglob("*"):
            if path.is_file():
                self.assertNotIn(b"private user state", path.read_bytes())

    def test_child_panic_and_launch_failure_are_retained(self):
        with self.assertRaisesRegex(AssertionError, "panicked"):
            with smoke.RunArtifacts(self.root) as evidence:
                with smoke.application([sys.executable, "-c", "print('thread panicked: synthetic')"], evidence) as app:
                    app.wait(timeout=5)
        self.assertEqual(self.result(evidence)["status"], "failed")
        self.assertIn("synthetic", (evidence.root / "app.log").read_text())
        with self.assertRaises(FileNotFoundError):
            with smoke.RunArtifacts(self.root) as missing:
                with smoke.application([str(self.root / "no-such-binary")], missing):
                    self.fail("unreachable")
        self.assertEqual(self.result(missing)["status"], "failed")
        self.assertTrue((missing.root / "app.log").exists())

    def test_shutdown_escalates_with_bounded_waits(self):
        app = Mock(pid=123, returncode=-9)
        app.poll.return_value = None
        app.wait.side_effect = [subprocess.TimeoutExpired("fixture", 5), -9]
        with smoke.RunArtifacts(self.root) as evidence:
            with patch.object(smoke.subprocess, "Popen", return_value=app):
                with smoke.application(["never-launched"], evidence):
                    pass
        app.terminate.assert_called_once_with()
        app.kill.assert_called_once_with()
        self.assertEqual([call.kwargs for call in app.wait.call_args_list], [{"timeout": 5}, {"timeout": 5}])

    def test_permission_gates_are_blocked_without_seeding_or_launching(self):
        for accessibility, screenshots in [(False, False), (True, True)]:
            with self.subTest(accessibility=accessibility):
                native = Mock()
                native.AXIsProcessTrusted.return_value = accessibility
                native.CGPreflightScreenCaptureAccess.return_value = False
                args = ["smoke", "--run", "--artifacts", str(self.root)]
                if screenshots:
                    args += ["--screenshots", str(self.root / "captures")]
                with patch.object(sys, "argv", args), patch.object(sys, "platform", "darwin"), \
                     patch.object(smoke.ctypes, "CDLL", return_value=native), \
                     patch.object(smoke, "seed") as seed_mock, \
                     patch.object(smoke.subprocess, "Popen") as popen, \
                     patch("sys.stderr", new_callable=io.StringIO):
                    self.assertEqual(smoke.main(), 2)
                seed_mock.assert_not_called()
                popen.assert_not_called()
        reports = [json.loads(path.read_text()) for path in self.root.glob("*/result.json")]
        self.assertEqual([report["status"] for report in reports], ["blocked", "blocked"])
        self.assertFalse((self.root / "captures").exists())

    def test_native_runner_records_early_exit_without_sending_native_input(self):
        native = Mock()
        native.AXIsProcessTrusted.return_value = True
        app = Mock(pid=123, returncode=17)
        app.poll.return_value = 17
        args = SimpleNamespace(binary=Path(sys.executable), screenshots=None)
        with self.assertRaisesRegex(AssertionError, "application exited early \\(17\\)"):
            with smoke.RunArtifacts(self.root) as evidence:
                fixture = seed(evidence.root)
                with patch.object(sys, "platform", "darwin"), \
                     patch.object(smoke.ctypes, "CDLL", return_value=native), \
                     patch.object(smoke, "seed", return_value=fixture), \
                     patch.object(smoke.subprocess, "Popen", return_value=app):
                    smoke.run_native(args, evidence)
        self.assertEqual(self.result(evidence)["status"], "failed")
        native.CGEventPost.assert_not_called()
        self.assertTrue((evidence.root / "failure-state/state.history.jsonl").is_file())
        self.assertTrue(any(row["description"] == "startup restored one Home tab" and row["event"] == "failed"
                            for row in self.observations(evidence)))

    def test_snapshot_copy_error_does_not_replace_the_original_failure(self):
        with self.assertRaisesRegex(AssertionError, "original failure"):
            with smoke.RunArtifacts(self.root) as evidence:
                (evidence.root / "app.log").write_text("still retained at the original path")
                with patch.object(smoke.shutil, "copyfile", side_effect=OSError("copy failed")):
                    evidence.snapshot_failure()
                raise AssertionError("original failure")
        self.assertEqual(self.result(evidence)["error"], "original failure")
        self.assertTrue(any(row["description"] == "failure snapshot" and row["error"] == "copy failed"
                            for row in self.observations(evidence)))
        self.assertEqual((evidence.root / "app.log").read_text(), "still retained at the original path")

    def test_help_does_not_check_permissions_or_create_artifacts(self):
        with patch.object(sys, "argv", ["smoke", "--artifacts", str(self.root)]), \
             patch.object(smoke.ctypes, "CDLL") as native:
            self.assertEqual(smoke.main(), 0)
        native.assert_not_called()
        self.assertEqual(list(self.root.iterdir()), [])

    def test_cli_success_and_failure_exit_codes_without_native_calls(self):
        for failure, code in [(None, 0), (AssertionError("synthetic timeout"), 1)]:
            with patch.object(sys, "argv", ["smoke", "--run", "--artifacts", str(self.root)]), \
                 patch.object(smoke, "run_native", side_effect=failure), \
                 patch("sys.stderr", new_callable=io.StringIO):
                self.assertEqual(smoke.main(), code)
        statuses = sorted(json.loads(path.read_text())["status"] for path in self.root.glob("*/result.json"))
        self.assertEqual(statuses, ["failed", "passed"])


class WaitTests(unittest.TestCase):
    def test_wait_retries_missing_state_and_transient_ax_errors(self):
        probe = Mock(side_effect=[None, subprocess.CalledProcessError(1, "osascript", output="window missing (-1719)"), 3])
        record = Mock()
        with patch.object(smoke.time, "sleep"):
            self.assertEqual(smoke.wait_for("ready", probe, lambda value: value == 3, record=record), 3)
        self.assertTrue(any(call.args[0] == "transient" for call in record.call_args_list))

    def test_wait_does_not_hide_unexpected_probe_or_predicate_errors(self):
        for error in [subprocess.CalledProcessError(1, "osascript", output="permission denied"), ValueError("bad state")]:
            record = Mock()
            with self.assertRaises(type(error)):
                smoke.wait_for("ready", Mock(side_effect=error), lambda value: True, record=record)
            self.assertEqual(record.call_args_list[-1].args[0], "failed")
        with self.assertRaisesRegex(ValueError, "bad predicate"):
            smoke.wait_for("ready", lambda: 3, Mock(side_effect=ValueError("bad predicate")))

    def test_stability_resets_when_geometry_changes_or_condition_is_false(self):
        stable = smoke.stable_for(lambda value: value > 0, seconds=0.5)
        with patch.object(smoke.time, "monotonic", side_effect=[0, 0.4, 0.6, 1.2, 1.8, 2, 2.6]):
            results = [stable(value) for value in [1, 1, 2, 2, 0, 2, 2]]
        self.assertEqual(results, [False, False, False, True, False, False, True])


if __name__ == "__main__":
    unittest.main()
