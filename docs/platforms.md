# Platform contract

## Supported development baseline

| Dimension | Tested baseline |
| --- | --- |
| OS | macOS **26.7.1 (25G241)** |
| Architecture | Native Apple Silicon: Rust `aarch64-apple-darwin`, Nix `aarch64-darwin` |
| Storage | Local internal **APFS, case-insensitive**, with ownership/permissions enabled; writable fixture and config directories |
| Compiler | Rust/Cargo **1.96.0**, pinned in `rust-toolchain.toml` and `flake.nix` |
| Native build | Xcode **27.1 (27A9275)**, macOS SDK **27.0**, command-line tools and Metal toolchain; the flake uses the host Xcode tools |
| Structural diff helper | Difftastic **0.71.0**, supplied by the locked Nix environment |

This is the supported source-development configuration, not a minimum OS version.
No older or newer macOS version has been qualified. An SDK deployment target is
not evidence that Kestrel runs on that OS. Xcode itself is host-installed, not
pinned by Nix; record its version with each build result.

The storage baseline applies to repositories (including their common Git
directories), workspace/history storage and test fixtures. It does not guarantee
delivery of every filesystem event: watchers invalidate cached Git metadata,
and reconciliation reads recover current refs, not intermediate ref history.
Comparisons remain pinned; platform verification must not change that policy.
Git storage-format compatibility is a separate contract from the filesystem.

## Unverified configurations

- Intel macOS and Rosetta execution.
- Other macOS versions, including possible lower deployment targets.
- Linux and Windows, on any architecture. Non-macOS native-adapter stubs are not
  implementations of the macOS appearance and window behavior.
- Case-sensitive APFS, external/removable volumes, HFS+, network filesystems,
  cloud-synchronized directories and other storage layouts. A local APFS probe
  cannot establish whether a directory is managed by a sync provider; use ordinary
  local directories for verification.

These are experimental source-porting targets, **not supported application
platforms**. The flake exposes only the verified `aarch64-darwin` development
shell; the former Intel/Linux shell declarations did not establish working
application ports. This does not add runtime rejection of other configurations.

## Repeatable verification matrix

Run from the checkout. Keep `Cargo.lock` and `flake.lock` unchanged. Use fresh,
local disposable fixture/config directories, never the real saved workspace.
Retain the source revision/diff, environment output, command logs and exit codes.
A failed or unavailable check is not a pass.

| Layer | Command | What it establishes |
| --- | --- | --- |
| Host and storage | `python3 scripts/check_platform.py` | Exact OS/build/architecture and checkout/temp-volume match; **not** application acceptance |
| Other fixture roots | `python3 scripts/check_platform.py /path/to/fixture-root /path/to/config-fixture-root` | Checks explicitly supplied existing writable directories, using a disposable case-sensitivity probe; no saved state is opened |
| Tool versions | `nix develop --command sh -ec 'rustc -vV; cargo -V; "$KESTREL_DIFFTASTIC" --version; xcodebuild -version; xcrun --sdk macosx --show-sdk-version'` | Record actual compiler host, helper, Xcode and SDK; compiler host must be `aarch64-apple-darwin` |
| Shell outputs | `nix eval --no-write-lock-file --json .#devShells --apply builtins.attrNames` | Only `aarch64-darwin` is provisioned |
| Harness regressions | `python3 -m unittest discover -s scripts -p 'test_*.py'` | Platform classification and native-smoke harness behavior, without launching the app |
| Formatting | `nix develop --command cargo fmt --all -- --check` | Source formatting |
| Lint | `nix develop --command cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Strict lint on the baseline target |
| Tests | `nix develop --command cargo test --locked --workspace --all-features` | Model, Git, persistence, GPUI test-context and native watcher regressions |
| Scheduling stress | `nix develop --command env ITERATIONS=20 cargo test --locked --workspace --all-features` | GPUI scheduling seeds, not twenty native GUI runs |
| Filesystem integration | `nix develop --command cargo run --locked -p kestrel --example watch_refs` | Real FSEvents and isolated Git fixtures: loose/packed refs, linked worktrees, disappearance and reattachment; no GUI |
| Debug build | `nix develop --command cargo build --locked --workspace --all-features` | Includes production native adapters, which test builds may stub |
| Release build | `nix develop --command cargo build --locked --release --workspace --all-features` | Optimized production compilation, not packaged launch |
| Native interaction | [Native navigation smoke](../README.md#native-navigation-smoke) | Separate, permission-dependent manual acceptance of an isolated build |

The preflight exits nonzero for an unverified environment or failed probe. Supply
all fixture roots if overriding the defaults; use the same temporary root for
the subsequent tests (for example via `TMPDIR`). It reports only selected volume
properties, not device identifiers. Successful preflight cannot prove compiler
target selection, watcher behavior, rendering, accessibility or release readiness.

## Verification evidence

On **2026-10-28 UTC**, the baseline preflight and shell-output check passed on the
configuration above. Formatting, strict all-target/all-feature Clippy, debug and
release builds passed. The all-feature Rust suite passed **352 tests**, with one
manual benchmark ignored, both normally and with 20 GPUI scheduling seeds. All
**27 Python harness tests** and the headless `watch_refs` example passed. The
existing `block 0.1.6` future-incompatibility warning remains.

No other platform or filesystem was tested. Historical
native navigation checks on the same OS do not certify the current tree; current
native acceptance must be recorded separately and remains blocked where its
permissions are unavailable. Headless checks do not certify VoiceOver, IME,
signing/notarization or launch outside the development environment.

To qualify another row, record its exact OS/build, native architecture, filesystem
and tool versions; run the automated matrix and native interaction checks on that
configuration. Resolve native-adapter gaps before claiming an application port.
Only then expand the documented support and flake outputs. Until that evidence
exists, report the row as unverified rather than selecting a speculative minimum.
