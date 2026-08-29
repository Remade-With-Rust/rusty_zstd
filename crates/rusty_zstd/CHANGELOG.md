# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.4](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.3...rusty_zstd-v0.2.4) - 2026-08-29

### Other

- an In the wild block above the headline ([#10](https://github.com/Remade-With-Rust/rusty_zstd/pull/10))

## [0.2.3](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.2...rusty_zstd-v0.2.3) - 2026-08-28

### Added

- *(encode)* ship DFast back-extension, and trade ~1% size for up to 38% encode speed

### Other

- name the release 0.2.3, which is what release-plz actually cuts

## [0.2.2](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.1...rusty_zstd-v0.2.2) - 2026-08-28

### Fixed

- *(encode)* a dispatch that never fired, and a knob with no reader

## [0.2.1](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.0...rusty_zstd-v0.2.1) - 2026-08-28

### Added

- *(alloc)* opt-in rusty-alloc feature that installs rusty_alloc_default
