#!/usr/bin/env python3
"""Check the documented development baseline, not application acceptance."""

import argparse
import json
from pathlib import Path
import platform
import plistlib
import subprocess
import tempfile


MACOS_VERSION = "26.7.1"
MACOS_BUILD = "25G241"


def host_gaps(system, machine, version, build):
    gaps = []
    if system != "Darwin" or machine != "arm64":
        gaps.append("requires native Apple Silicon macOS (arm64), not Rosetta")
    if (version, build) != (MACOS_VERSION, MACOS_BUILD):
        gaps.append(f"tested macOS is {MACOS_VERSION} ({MACOS_BUILD}); no minimum is established")
    return gaps


def volume_gaps(info, case_sensitive):
    gaps = []
    if info.get("FilesystemType") != "apfs" or info.get("Internal") is not True:
        gaps.append("requires local internal APFS; other storage is unverified")
    if info.get("WritableVolume") is not True:
        gaps.append("requires a writable fixture volume")
    if info.get("GlobalPermissionsEnabled") is not True:
        gaps.append("requires filesystem ownership/permissions enabled")
    if case_sensitive:
        gaps.append("case-sensitive filesystems are unverified")
    return gaps


def output(*args):
    return subprocess.check_output(args, stderr=subprocess.PIPE, timeout=15)


def inspect_volume(path):
    path = path.resolve(strict=True)
    # df resolves the actual data volume, including symlinks and APFS firmlinks.
    device = output("/bin/df", "-P", str(path)).decode().splitlines()[-1].split()[0]
    info = plistlib.loads(output("/usr/sbin/diskutil", "info", "-plist", device))
    if not isinstance(info, dict):
        raise ValueError("diskutil did not return volume metadata")
    with tempfile.TemporaryDirectory(prefix="kestrel-platform-", dir=path) as fixture:
        lower = Path(fixture) / "case-probe"
        lower.write_bytes(b"")
        case_sensitive = not lower.with_name("CASE-PROBE").exists()
    return {
        "path": str(path),
        "filesystem": info.get("FilesystemType"),
        "internal": info.get("Internal"),
        "writable": info.get("WritableVolume"),
        "ownership_enabled": info.get("GlobalPermissionsEnabled"),
        "case_sensitive": case_sensitive,
        "gaps": volume_gaps(info, case_sensitive),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths", nargs="*", type=Path,
        help="existing writable fixture roots (default: checkout and temporary directory)",
    )
    args = parser.parse_args(argv)
    system, machine = platform.system(), platform.machine()
    result = {"system": system, "architecture": machine, "volumes": [], "gaps": []}
    try:
        version = output("/usr/bin/sw_vers", "-productVersion").decode().strip() if system == "Darwin" else ""
        build = output("/usr/bin/sw_vers", "-buildVersion").decode().strip() if system == "Darwin" else ""
        result.update(macos_version=version, macos_build=build)
        result["gaps"].extend(host_gaps(system, machine, version, build))
        if system == "Darwin":
            for path in args.paths or [Path.cwd(), Path(tempfile.gettempdir())]:
                volume = inspect_volume(path)
                result["volumes"].append(volume)
                result["gaps"].extend(volume["gaps"])
    except (OSError, ValueError, IndexError, plistlib.InvalidFileException, subprocess.SubprocessError) as error:
        result["gaps"].append(f"unable to verify baseline: {error}")
    result["status"] = "unverified" if result["gaps"] else "baseline_match"
    print(json.dumps(result, indent=2))
    return 1 if result["gaps"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
