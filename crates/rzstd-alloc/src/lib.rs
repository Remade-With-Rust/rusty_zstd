//! Allocator seam for rusty_zstd binaries.
//!
//! House law: `#[global_allocator]` lives in the *deliverable* (`main.rs`),
//! never in a shared library. This crate holds the exact `rusty_alloc-api`
//! pin so feature code never names that crate.
//!
//! PIN: `=2.0.5` (was `=2.0.0`, and `=1.1.4` before that). The doc here once
//! said `=1.1.0` while the manifest said `=1.1.4` -- a stale comment beside
//! the thing it documents, which is the one place a pin must not drift. Read
//! the version from `Cargo.toml`.
//!
//! KNOWN SPLIT, deliberate: `rusty_zstd`'s optional `rusty-alloc` feature goes
//! through `rusty_alloc_default`, which as of 0.1.2 still tracks the 1.x line
//! (1.1.6). So the CLI and bench binaries run rusty_alloc 2.0.5 through THIS
//! seam while that feature would install 1.1.6. No single binary links both --
//! `cargo tree` on the CLI shows only 2.0.5 -- but the two paths are on
//! different majors until `rusty_alloc_default` publishes a 2.x.

#![no_std]

pub use rusty_alloc_api::RustyAlloc as Alloc;
