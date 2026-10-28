# TLS dependency advisories

`Cargo.lock` updates `rustls` from 0.23.45 to 0.23.46 and `rustls-webpki`
from 0.103.15 to 0.103.16. Existing dependency constraints permit both patches;
no manifest or application behavior changes are needed.

## Applicability

The dependency path is `gpui → gpui_http_client → zed-reqwest → rustls`,
including the `hyper-rustls` and `tokio-rustls` adapters, then `rustls-webpki`.
On `aarch64-apple-darwin`, default and all-workspace-feature resolution enables
`rustls` features `ring,std,tls12` and `rustls-webpki` features `alloc,ring,std`.
Rustls's Brotli decompressor and AWS-LC provider are not enabled.

Kestrel currently inspects local Git data. `crates/kestrel/src/main.rs` uses
`Application::new()`, which supplies GPUI 0.2.2's `NullHttpClient`; Kestrel does
not replace it. No application HTTP requests, custom TLS session store, ECH
configuration, or post-quantum network policy were found. These advisory
matches are not demonstrated application exploits.

| Advisory | Applicability before the update |
| --- | --- |
| [RUSTSEC-2026-0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html) | Debug formatting of client session values can expose TLS secrets. No Kestrel session-store implementation or corresponding logging was found; ordinary built-in cache debug output is not affected. |
| [RUSTSEC-2026-0347](https://rustsec.org/advisories/RUSTSEC-2026-0347.html) | Requires enabled ECH, pre-handshake application writes, and server rejection of the ECH configuration. No such Kestrel configuration or callsite was found. |
| [RUSTSEC-2026-0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html) | Excessive Brotli certificate-decompression allocation does not apply to the inspected macOS feature resolution: `brotli` is disabled. |
| [RUSTSEC-2026-0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html) | Unauthenticated key-share prediction can downgrade later connections' post-quantum confidentiality. The inspected provider is ring, with no Kestrel PQ policy. This is not an authentication bypass. |
| [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382.html) | Distrusted emSign intermediates may be accepted when trust anchors permit the path. The transitive HTTP client supports native trust roots, but no Kestrel TLS connection was found. The patch rejects the distrusted keys. |

Revisit reachability when adding networking, replacing GPUI's HTTP client, or
changing dependency features. Patched versions avoid relying on today's lack
of application networking.

## Verification

With cargo-audit 0.22.2 and RustSec database revision
`3279c99911354c400e1d41e47c7d60e39258b147`, auditing the previous lockfile
reported these five vulnerabilities (exit 1); auditing the updated lockfile
reported zero (exit 0). The existing three unsoundness warnings and thirteen
unmaintained-package entries remain separate follow-up work, not suppressed
advisories. Audit success is not a claim that all dependencies are risk-free.

To repeat the advisory and feature checks inside the Nix development shell
(with cargo-audit available):

```sh
cargo audit
cargo tree --locked --target aarch64-apple-darwin -i rustls -e features
cargo tree --locked --target aarch64-apple-darwin --all-features -i rustls -e normal --format '{p} {f}'
cargo tree --locked --target aarch64-apple-darwin --all-features -i rustls-webpki -e normal --format '{p} {f}'
```

For a comparison against a fixed local RustSec database, use
`cargo audit --no-fetch --db <database> --file <lockfile> --json` on both
lockfiles. No advisory exceptions are needed.

Headless verification with Rust 1.96.0 on Apple Silicon macOS passed:

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
ITERATIONS=20 cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo build --locked --workspace
cargo build --locked --workspace --release
```

Each test run passed 363 tests, with one manual benchmark ignored. The existing
`block` future-incompatibility warning remains.
