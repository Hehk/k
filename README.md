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

Enable the version-controlled pre-commit hook once per clone:

```sh
git config --local core.hooksPath .githooks
```

Commit from the dev environment. The hook checks formatting, Clippy, and tests, stopping on the first failure. It checks the working tree, including unstaged changes, without modifying files.
