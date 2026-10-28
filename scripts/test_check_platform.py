import contextlib
import io
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import check_platform as check


VOLUME = {
    "FilesystemType": "apfs",
    "Internal": True,
    "WritableVolume": True,
    "GlobalPermissionsEnabled": True,
}


class PlatformTests(unittest.TestCase):
    def test_exact_baseline(self):
        self.assertEqual(check.host_gaps("Darwin", "arm64", "26.7.1", "25G241"), [])
        self.assertEqual(check.volume_gaps(VOLUME, False), [])

    def test_no_other_os_architecture_or_version_is_promoted(self):
        for host in [
            ("Linux", "aarch64", "", ""),
            ("Windows", "ARM64", "", ""),
            ("Darwin", "x86_64", "26.7.1", "25G241"),
            ("Darwin", "arm64", "26.7", "25G241"),
            ("Darwin", "arm64", "27.0", "26A1"),
            ("Darwin", "arm64", "26.7.1", "different-build"),
        ]:
            with self.subTest(host=host):
                self.assertTrue(check.host_gaps(*host))

    def test_unverified_or_unknown_volume_properties_fail_closed(self):
        for field, value in [
            ("FilesystemType", "hfs"),
            ("FilesystemType", "smbfs"),
            ("Internal", False),
            ("WritableVolume", False),
            ("GlobalPermissionsEnabled", False),
            *[(key, None) for key in VOLUME],
        ]:
            with self.subTest(field=field, value=value):
                self.assertTrue(check.volume_gaps({**VOLUME, field: value}, False))
        self.assertTrue(check.volume_gaps(VOLUME, True))

    def test_volume_probe_resolves_symlink_and_removes_only_its_fixture(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            storage = root / "storage with spaces"
            storage.mkdir()
            sentinel = storage / "keep"
            sentinel.write_text("untouched")
            link = root / "link"
            link.symlink_to(storage, target_is_directory=True)
            with patch.object(check, "output", side_effect=[
                b"Filesystem blocks used available capacity Mounted on\n/dev/disk9s1 1 1 1 1% /volume with spaces\n",
                plistlib.dumps(VOLUME),
            ]) as output:
                report = check.inspect_volume(link)
            self.assertEqual(report["path"], str(storage.resolve()))
            self.assertIsInstance(report["case_sensitive"], bool)
            self.assertEqual(list(storage.iterdir()), [sentinel])
            self.assertEqual(sentinel.read_text(), "untouched")
            self.assertEqual(output.call_args_list[0].args, ("/bin/df", "-P", str(storage.resolve())))
            self.assertEqual(output.call_args_list[1].args, ("/usr/sbin/diskutil", "info", "-plist", "/dev/disk9s1"))

    def test_malformed_volume_metadata_cannot_match_the_baseline(self):
        with tempfile.TemporaryDirectory() as root:
            for metadata in [b"not a plist", plistlib.dumps([])]:
                with self.subTest(metadata=metadata), patch.object(check, "output", side_effect=[
                    b"Filesystem blocks used available capacity Mounted on\n/dev/disk9s1 1 1 1 1% /volume\n",
                    metadata,
                ]):
                    with self.assertRaises((ValueError, plistlib.InvalidFileException)):
                        check.inspect_volume(Path(root))
                self.assertEqual(list(Path(root).iterdir()), [])

    def run_main(self, system="Darwin", machine="arm64", responses=None, volume=None):
        with patch.object(check.platform, "system", return_value=system), \
             patch.object(check.platform, "machine", return_value=machine), \
             patch.object(check, "output", side_effect=responses), \
             patch.object(check, "inspect_volume", return_value=volume) as inspect, \
             contextlib.redirect_stdout(io.StringIO()) as stdout:
            code = check.main(["fixture"])
        return code, json.loads(stdout.getvalue()), inspect

    def test_matching_preflight_does_not_claim_application_pass(self):
        code, result, inspect = self.run_main(
            responses=[b"26.7.1\n", b"25G241\n"], volume={"gaps": []},
        )
        self.assertEqual(code, 0)
        self.assertEqual(result["status"], "baseline_match")
        inspect.assert_called_once_with(Path("fixture"))

    def test_linux_does_not_run_macos_commands_or_inspect_volumes(self):
        code, result, inspect = self.run_main(system="Linux", machine="aarch64", responses=AssertionError())
        self.assertEqual(code, 1)
        self.assertEqual(result["status"], "unverified")
        inspect.assert_not_called()

    def test_volume_gap_produces_nonzero_exit(self):
        code, result, _ = self.run_main(
            responses=[b"26.7.1", b"25G241"], volume={"gaps": ["unverified storage"]},
        )
        self.assertEqual(code, 1)
        self.assertIn("unverified storage", result["gaps"])

    def test_failed_or_timed_out_probe_is_not_a_pass(self):
        for error in [OSError("missing command"), subprocess.TimeoutExpired("sw_vers", 15)]:
            with self.subTest(error=error):
                code, result, _ = self.run_main(responses=error)
                self.assertEqual(code, 1)
                self.assertEqual(result["status"], "unverified")
                self.assertIn("unable to verify baseline", result["gaps"][0])


if __name__ == "__main__":
    unittest.main()
