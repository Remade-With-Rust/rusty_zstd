//! Present only because Cargo requires a build script on any package that sets
//! `links`. There is nothing native to probe here: the `links = "rusty_alloc_global"`
//! key exists to make Cargo reject a dependency graph containing two allocator
//! seams, which it enforces at resolve time from the manifest alone.

fn main() {
    // Nothing to detect, and nothing to emit. Re-run only if this file changes,
    // so the empty script never invalidates the build cache.
    println!("cargo:rerun-if-changed=build.rs");
}
