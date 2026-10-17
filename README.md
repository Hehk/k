# Kestrel

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
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Structural diffs

There is one diff view: Difftastic 0.71.0 compares supported files, with automatic libgit2 fallback when structural comparison is unavailable or fails. The label above the diff identifies the engine or fallback reason. Both use the same line colors and typography; structural token highlights add subtle background emphasis, not bold text. Formatting-only changes remain in the file list with a “No structural changes” notice, and missing-final-newline metadata remains visible.

`nix develop` provides the pinned helper and sets `KESTREL_DIFFTASTIC`. The build also records that Nix store path, so the resulting executable can find it outside the shell. Lookup order is an explicit absolute `KESTREL_DIFFTASTIC` path, a `difft` executable beside Kestrel, then the build-time path. Missing or incompatible helpers fall back to text. A distributable app must package the helper and its license notices; signing and distribution are not implemented yet.

`structural_diff` exposes source buffers, aligned source lines, and changed byte ranges. Its private backend owns JSON decoding, temporary files, and process supervision. Git access and GPUI stay outside that boundary, allowing a later in-process implementation without changing the viewer. Comparisons use an isolated child environment, bounded output, a one-second execution budget, and one helper at a time. No Git settings are changed.
