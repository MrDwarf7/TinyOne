---
title: Authority Registry Recovery Gate
---

# Authority Registry Recovery Gate

The `recovery/project-shape-integration-20260829` commit contains local WIP
for Authority-backed registry modules, canonical signing formats, and a build
script that embeds a separately protected TUF initial root. The Authority
crates required by that WIP are intentionally not workspace dependencies:

- `tinyone-module-client`
- `tinyone-repository-core`
- `tinyone-signing-format`
- `tinyone-signing-interface`

The recovered source now lives in the `tinyone_core` crate:

- `src/authority_registry_wip/config.rs` preserves the registry configuration
  and signing-format changes.
- `src/authority_registry_wip/modules.rs` preserves eager registry refresh,
  verified materialization, cache-input tracking, and the authority suspension
  integration test.
- `src/authority_registry_wip/build.rs` preserves generation of the embedded
  TUF initial-root source.

## Deliberately inert

The `authority-registry-wip` feature is a recovery label, not an activation
mechanism. Every related module is gated by both that feature and `cfg(any())`.
`cfg(any())` is always false, so the Authority-dependent source cannot compile,
including under `cargo check --all-features` or `cargo test --all-features`.
No Authority path dependency is declared in any manifest, which keeps a fresh
Windows or Linux checkout buildable without an adjacent Authority repository.

Do not weaken the second gate merely to experiment with this code. Re-enabling
it requires a reviewed supply of all four Authority crates, reconciliation with
the current `config` and resolver APIs, and Windows/Linux TUF, revocation, ABI,
and C-consumer validation. Until then, TinyOne retains its existing signed
project-module behavior and does not accept Authority registry configuration at
runtime.
