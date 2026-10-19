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

Difftastic 0.71.0 supplies comparison data to the existing diff view, with automatic libgit2 fallback when structural comparison is unavailable or fails. Layout, gutters, line styling, selection, and scrolling are unchanged. Hover over the file title for the engine or fallback reason. Formatting-only files and missing-final-newline metadata remain visible.

`nix develop` provides the pinned helper and sets `KESTREL_DIFFTASTIC`. The build also records that Nix store path, so the resulting executable can find it outside the shell. Lookup order is an explicit absolute `KESTREL_DIFFTASTIC` path, a `difft` executable beside Kestrel, then the build-time path. Missing or incompatible helpers fall back to text. A distributable app must package the helper and its license notices; signing and distribution are not implemented yet.

The [`diff`](crates/diff) workspace crate exposes source comparison, aligned lines, changed byte ranges, limits, and cancellation. Its private backend owns the Difftastic executable, JSON validation, temporary files, and process supervision. The app owns Git access, text fallback, and rendering; the crate has no runtime Git or GPUI dependencies.

Comparisons use an isolated environment, bounded output, a one-second execution budget, and one helper at a time. Rust's `Child::kill` and `wait` supervise only the launched child, not process trees; custom helpers must honor that single-process contract. No Git settings are changed.

Test the crate independently with `nix develop -c cargo test -p diff`.
