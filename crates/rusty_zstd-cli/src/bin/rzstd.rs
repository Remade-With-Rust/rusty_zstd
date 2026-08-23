//! `rzstd` -- the primary name. The mode is read off `argv[0]` inside `entry()`.
//!
//! House law: `#[global_allocator]` lives in the deliverable, never in a
//! library, so each shim installs it rather than the crate's `lib.rs`.

#[global_allocator]
static ALLOC: rzstd_alloc::Alloc = rzstd_alloc::Alloc;

fn main() -> std::process::ExitCode {
    rusty_zstd_cli::entry()
}
