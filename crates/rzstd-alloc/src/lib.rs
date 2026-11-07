//! Allocator seam for rusty_zstd binaries.
//!
//! House law: `#[global_allocator]` lives in the *deliverable* (`main.rs`),
//! never in a shared library. This crate holds the exact `rusty_alloc-api`
//! pin so feature code never names that crate.
//!
//! PIN: read the version from `Cargo.toml` (`=2.4.0` as of 0.4.2; before that
//! `=2.2.6`, `=2.2.5`, `=2.0.5`, `=2.0.0` and `=1.1.4`). This comment once
//! named a different pin than the manifest -- a stale comment beside the thing
//! it documents, which is the one place a pin must not drift.
//!
//! `rusty_zstd`'s optional `rusty-alloc` feature installs the allocator
//! through `rusty_alloc_default` instead (so several crates enabling the
//! feature still link one `#[global_allocator]`); it pins the same
//! `rusty_alloc-api` release as this seam.

#![no_std]

pub use rusty_alloc_api::RustyAlloc as Alloc;
