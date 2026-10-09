# Top-bar property-testing experiment

This branch is one of four alternatives: Hegel, Proptest, QuickCheck, and Bolero. Do not merge all four.

Start with **`generated.rs`** to review the library-specific code. **`contracts.rs`** contains the shared behavioral oracles; **`mod.rs`** contains input types, invariants, tracing, and JSON replay. The shared files are identical across alternatives. Production behavior and the original 53 tests are unchanged.

## Run

From this worktree's root:

```sh
nix develop
cargo test -p kestrel top_bar::model::properties
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The six generated properties default to 256 cases each. There are also ordinary contract examples and an optional JSON replay test. Each worktree has its own build directory; dependency caches may be reused, but tests must run against that worktree's source.

| Library | Larger run | Seed for investigation |
| --- | --- | --- |
| Hegel 0.49.2 | `HEGEL_TEST_CASES=4096 cargo test properties::generated` | `HEGEL_SEED=7` |
| Proptest 1.11.0 | `PROPTEST_CASES=4096 cargo test properties::generated` | `PROPTEST_RNG_SEED=7` |
| QuickCheck 1.1.0 | `QUICKCHECK_TESTS=4096 cargo test properties::generated` | `KESTREL_PBT_SEED=7` |
| Bolero 0.13.7 | `KESTREL_PBT_CASES=4096 cargo test properties::generated` | Use its reported failure seed/artifact or JSON replay below. |

Equal seeds do not produce equal inputs across libraries. Defaults retain fresh exploration. For accepted-step and transition counts, set `KESTREL_PBT_STATS=1` and append `-- --nocapture --test-threads=1`; each successful case prints one `PBT_STATS` JSON line. Setup messages are excluded from those counters.

The supported development environment is the existing pinned Rust 1.96/Nix shell. The package's declared Rust 1.85 minimum is not newly verified by this experiment. Published Proptest 1.11.0 declares 1.85 (unlike the newer upstream README); Hegel 0.49.2 requires 1.86 for test tooling. Hegel's default build compiles a separate engine shared library and loads it at runtime. Ordinary Bolero tests need no `cargo-bolero` installation; fuzz-engine runs are not part of the comparison.

## Replay a failure

Failures include a `REPLAY_JSON=...` line, the materialized message/command trace, and the model at the failure. Copy the JSON after the equals sign to a file, then:

```sh
KESTREL_PBT_REPLAY=/absolute/path/case.json \
  cargo test top_bar::model::properties::replay -- --exact --nocapture
```

This bypasses random generation and shrinking and works in every alternative. It replays the semantic input, resolving references deterministically; the printed `TRACE` exposes the actual messages. Preserve the library's own reduced witness/seed as well. When debugging a planted fault, replay against the same fault; the same input must pass once that fault is removed.

## Contracts and domains

| Property | Oracle and surface |
| --- | --- |
| `lifecycle` | Simple list/selection model for add, select, numbered/relative selection, close/close-active, hover, background, and missing selection. Checks ordered non-frame effects and projected visibility after every step. |
| `drag_lifecycle` | Independent list reorder using known 100px fixture slots; membership/selection preserved, stationary samples ignored, release/cancel restores visibility and focus commands. |
| `detach_protocol` | Retains source until correlated success, preserves it on failure, checks request payload/order, neighbor selection, single-tab close, duplicates, and late replies during another detach. |
| `rejected_events` | Missing IDs, foreign/stale tokens, busy-phase actions, reused sessions, and terminal closing are unchanged-state/no-command checks. |
| `animation` | Controlled frame times, stale/backwards/duplicate frames, logical state/geometry isolation, resize retarget continuity, and settled projection. |
| `geometry` | Independent f64 rectangle scan for bounded integer pointer probes; exact half-open boundaries, gaps, clipping, and translation on an integer grid. |

All `Msg` variants are exercised. Construction covers `with_tab`; the unchanged example tests cover `new`. `view`, `active_tab`, `tab_index`, `next_drag`, `drag_token`, and `geometry` are checked through these contracts. Existing examples retain midpoint animation and resize/scroll-during-drag cases; this is not generated coverage of arbitrary mixed message interleavings.

Inputs:

- 1–16 initial tabs, arbitrary source identity, transferred IDs in the `u32` range, and titles of up to 24 Unicode characters.
- 0–128 lifecycle operations or drag target ranks. A rank resolves against current reference order, so deletion during shrinking does not leave dangling IDs. Numbered selection uses the raw index and can miss. Terminal close ends a lifecycle sequence; rejection after close is tested separately.
- Up to 144 live tabs in an all-add sequence. This deliberately replaces the proposed 32-tab cap rather than silently discarding add operations.
- Geometry widths/scroll magnitudes 0–4096px, origins −1024–1024px, pointer x −256–4096px and y −64–128px. Reorder fixtures use exactly representable 100px slots; geometry probes separately explore overflow and scroll clipping.
- Animation draws 0–319ms, maps to a progressing 1–159ms sample, then explicitly checks completion. A 0.001px continuity tolerance is fixed in advance for bounded f32 arithmetic, not adjusted after a failure.

These are exploration budgets, not product limits. Nonfinite geometry, identifier exhaustion, huge collections, native rendering/events, and true cross-window effects are outside the generated domain. The model can transfer its last tab; the UI intentionally cannot start a single-tab drag. A green suite is evidence only for the sampled domain.

## Review method

Compare native generation/shrinking code, not just line counts or execution speed. Distributions differ even with equal domains and case counts. The same four fault patches (wrong close neighbor, invalid token acceptance, premature detach removal, slot boundary) are tested in disposable worktrees. Findings and reduced witnesses are reported separately from real product bugs. No mutant belongs in a PR's production code.

Guidance: [Hegel writing/review skills](https://github.com/hegeldev/hegel-skill) and [Trail of Bits property-testing skill](https://github.com/trailofbits/skills/tree/main/plugins/property-based-testing). We use constructive generation and independent oracles, not generic claims that UI events commute.
