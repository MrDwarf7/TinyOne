//! Dormant recovery source for the Authority-backed module-registry WIP.
//!
//! `authority-registry-wip` documents the intended future integration point,
//! but the `any()` half of each gate is intentionally always false. Therefore
//! these modules cannot compile—not even with `cargo --all-features`—while the
//! Authority crates are unavailable. The complete recovery implementations
//! live next to this file so no external path dependency is required.

#[cfg(all(feature = "authority-registry-wip", any()))]
mod config {
    include!("authority_registry_wip/config.rs");
}

#[cfg(all(feature = "authority-registry-wip", any()))]
mod modules {
    include!("authority_registry_wip/modules.rs");
}
