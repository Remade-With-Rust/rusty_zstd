# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/Remade-With-Rust/rusty_zstd/compare/rusty_zstd-v0.2.5...rusty_zstd-v0.3.0) - 2026-10-08

**Encode is 1.8x faster.** On the standing board (dickens, samba, x-ray,
mozilla, nci and xml at L1/3/5/7/9/12, whole files, one core, best of five
alternating rounds) encode time against 0.2.5 is:

| L1 | L3 | L5 | L7 | L9 | L12 | geomean |
|---|---|---|---|---|---|---|
| 0.61x | 0.72x | 0.68x | 0.61x | 0.54x | 0.31x | **0.56x** |

**Compressed output changes at L4-L12** -- smaller, and still plain zstd
frames: libzstd 1.5.7 decodes them byte-for-byte. L1-L3 and L13 and up are
byte-identical to 0.2.5. Decoding is unchanged.

### Changed -- output

- *(rowfind)* L5-L12 move from the hash chain to libzstd's ROW match finder
  with its lazy parse: 16/32/64-slot rows by search log, 8-bit hash tags,
  the 8-ahead hash cache, rows that fill downward. L6 runs it for sources of
  1 MiB or more; L4 runs it where the level table makes L4 greedy (16-256
  KiB sources).
- Size, 18 corpora capped at 4 MiB, against 0.2.5: L5 -1.8%, L7 -2.8%, L9
  -2.9%, L12 -2.9% (reymont and dickens -9..-11%, nci and xml -9..-12%,
  versions -37%). One corpus grows: `smallmsg-8m`, +2.0..+3.6% at L5-L12
  (record-periodic messages; libzstd's own row greedy lands in the same
  place). Positions a repcode match covers are not indexed, a departure from
  libzstd measured on `versions-16m` (-74% at 1 MiB).

### Changed -- speed, byte-identical

- *(fast, dfast)* L1-L4: the Fast pair and pipelined routes run in their own
  scan loops with one multiply per key; DFast keeps candidates in the table's
  encoding and reuses the speculation's long candidate.
- *(huf)* literals are emitted in libzstd's shape (7 instructions per literal,
  from 16), coded straight into the output, with a length limiter that no
  longer rescans the alphabet per step.
- *(seq)* the sequence coder writes one merged extra-bits field per sequence
  from a fixed flush schedule, with branch-free repcode steps.
- *(dict)* a dictionary is primed once per (dictionary, parameters) and
  restored per call, instead of re-primed every call.
- *(alloc)* the CLI and bench binaries move to rusty_alloc 2.2.5 (from 2.0.5);
  measured neutral (0.98-1.00x). The optional `rusty-alloc` feature still
  installs 1.1.6 through `rusty_alloc_default` 0.1.2.
- *(encode)* `encode.rs` split into modules.

### Added

- `RZSTD_ROW*` environment arms and `set_row_*_arm` bench hooks for every
  row-finder decision above (hidden, no semver promise); `RZSTD_ROW=0`
  returns L5-L12 to the hash chain.

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
