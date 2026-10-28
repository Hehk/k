"""Headless Git fixture regressions; no native application is launched."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

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


if __name__ == "__main__":
    unittest.main()
