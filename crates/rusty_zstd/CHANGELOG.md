# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.6](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.5...rusty_zstd-v0.2.6) - 2026-10-04

Compressed output is byte-identical to 0.2.5 at every level: everything new
below ships behind arms that default off.

### Added

- *(rowfind)* the row match finder takes a row width (16 / 32 / 64 slots),
  a table size and a policy as parameters (`RZSTD_ROW_LOG`,
  `RZSTD_ROW_SIZING`, `RZSTD_ROW_POLICY`)
- *(rowfind)* libzstd's lazy parse for row frames (`RZSTD_ROW_PARSE`), with
  a wide row key (`RZSTD_ROW_WIDE`), the long-match fill skip
  (`RZSTD_ROW_SKIP`) and a rule that leaves repcode-covered positions out of
  the table (`RZSTD_ROW_REPSKIP`). Together, against the shipped hash chain:
  0.87x time / -3.5% size at L7, 0.73x / -4.1% at L9, 0.56x / -3.7% at L12
  (six silesia files); not yet the default
- bench hooks `set_row_geom_arm`, `set_row_policy_arm`, `set_row_wide_arm`,
  `set_row_skip_arm`, `set_row_parse_arm`, `set_row_repskip_arm`,
  `set_row_reptake_arm`, `set_row_nice_arm`, `set_row_pf_arm`

### Changed

- *(rowfind)* the row walk gathers every candidate and prefetches its source
  line before the first compare, and names the next search's row as soon as
  a match is chosen: 11-14% faster on row frames, same output
- *(encode)* `encode.rs` split into seven modules; the asm board is identical
  in all thirty-two columns (#18)

## [0.2.5](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.4...rusty_zstd-v0.2.5) - 2026-09-09

### Fixed

- *(no_std)* the census counters no longer cost the crate bare metal

## [0.2.4](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.3...rusty_zstd-v0.2.4) - 2026-09-09

### Added

- *(encode)* 1.08-1.27x faster matchfind at L3-L12, byte-identical

### Fixed

- *(bench,test)* two defects CI found the moment the files became visible

### Other

- Merge pull request #12 from Remade-With-Rust/perf/matchfind-campaign

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
