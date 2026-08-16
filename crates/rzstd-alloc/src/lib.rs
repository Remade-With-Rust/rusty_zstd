//! Allocator seam for rusty_zstd binaries.
//!
//! House law: `#[global_allocator]` lives in the *deliverable* (`main.rs`),
//! never in a shared library. This crate holds the exact `rusty_alloc-api`
//! pin (`=0.4.0`) so feature code never names that crate.

#![no_std]

pub use rusty_alloc_api::RustyAlloc as Alloc;
