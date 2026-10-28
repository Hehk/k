# Kestrel

## Platform support

The supported development baseline is **native Apple Silicon macOS 26.7.1
(25G241), on local, internal, case-insensitive APFS**. This is a tested
configuration, not a minimum macOS version or a release-readiness claim.
Intel macOS, Rosetta, Linux, Windows, other macOS versions and other filesystems
are unverified. The Nix flake provides only `aarch64-darwin`; a development shell
is not an application port.

See [the platform contract and verification matrix](docs/platforms.md) for
headless checks, native acceptance limits and how to qualify another platform.

## Development

On macOS, install Xcode and its command-line tools. GPUI compiles Metal shaders, so the separate Metal toolchain component must also be installed if Xcode has not installed it already:

```sh
xcodebuild -downloadComponent MetalToolchain
```

Nix is used to manage the dev environment

```sh
nix develop
cargo run -p kestrel
```

The flake selects the installed Xcode Metal toolchain for GPUI's `xcrun` shader build. Useful checks:

```sh
python3 scripts/check_platform.py
cargo test --locked --workspace --all-features
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

### Native navigation smoke

Test the harness without launching the app or requiring permissions:

```sh
python3 -m unittest discover -s scripts -p 'test_macos_navigation_smoke.py'
```

For manual macOS acceptance after building, run:

```sh
python3 scripts/macos_navigation_smoke.py --run --binary target/debug/kestrel --artifacts /tmp/kestrel-smoke
```

This brings fixture windows forward and requires Accessibility permission; it does not change settings. Missing permissions produce a **blocked**, not passing, result.

Every run retains a private, unique directory with `result.json` and timestamped `observations.jsonl`. Once seeded and launched, it also contains the synthetic Git repository, workspace/history, and full `app.log`. Failures retain `harness.log` and available pre-cleanup files under `failure-state/`. The printed directory is never automatically deleted; remove it when finished. Collection is limited to the fixture, without copying your saved workspace or environment. Review artifacts before sharing.

Optional `--screenshots /tmp/kestrel-captures` requires Screen Recording permission and saves only the fixture application's window in a private subdirectory. Captures require manual visual review. State and window-geometry checks use bounded polling; custom popup readiness and title-bar hit targets lack AX probes, so remaining input pacing and coordinate assumptions are explicitly reported as unsupported checks. Stable geometry is not proof of completed animation. Neither the headless tests nor this smoke test verify VoiceOver support.
