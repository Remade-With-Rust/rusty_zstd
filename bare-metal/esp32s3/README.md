# rusty_zstd on an ESP32-S3 — the on-board proof

"Builds for a bare-metal target" and "runs on the part" are different claims.
CI makes the first one every push (`cargo check --target thumbv7em-none-eabihf`
and `--target riscv32imac-unknown-none-elf`). This directory is the second one,
and it is hand-run: it needs Espressif's Rust fork, which CI does not have.

**Result, 2026-09-09, ESP32-S3 revision v0.2, 8 MB flash, 192 KiB heap:**

```
=== rusty_zstd on ESP32-S3 (xtensa, no_std + alloc) ===
census64::CENSUS_LIVE = false  (false is expected here: no 64-bit atomics)
source            8260 bytes
L1  8260 ->  468 bytes  (17.65x)  round trip OK
L3  8260 ->  426 bytes  (19.39x)  round trip OK
L5  8260 ->  263 bytes  (31.41x)  round trip OK

RESULT: PASS -- compressed and decompressed on the board
```

Three levels because they are three different match finders: L1 is Fast, L3 is
DFast, L5 is Greedy. Each compressed and then decompressed back to a buffer
compared byte for byte against the source.

## Why this part is the right test

Xtensa LX7 is 32-bit, so `core::sync::atomic::AtomicU64` does not exist on it —
the same reason Cortex-M4F and RV32 could not build the crate at all until
`census64` landed. Every census counter here is the zero-sized stub, which is
why `CENSUS_LIVE` prints `false`. **A zero from a counter on this part means
"not measurable on this target", never "measured zero".**

The board therefore exercises the exact configuration the fix created, and the
`PASS` line is the evidence that stubbing the instrument did not disturb the
codec.

## Running it

```sh
espup install                      # once: the `esp` Rust fork for Xtensa
cargo install espflash             # once
cd bare-metal/esp32s3
cargo run --release -- --port COM4 # your port; omit on Linux/macOS to autodetect
```

Passes when the last line reads `RESULT: PASS`. It is not in the workspace
(`exclude` in the root manifest), so a normal `cargo build` at the repo root
never sees it and never needs the Xtensa toolchain.

## What it does not claim

- **No timing.** There is no cycle count here, so nothing in this directory is a
  performance claim. It answers "does it work", not "how fast".
- **Heap, not measured to a floor.** 192 KiB was enough for an 8 KiB source at
  levels 1 to 5. The crate sizes its tables from the SOURCE length, so a bigger
  input needs more; finding the minimum heap per level is a separate exercise.
- **One part.** The S3 is Xtensa. The two targets CI compiles are ARM and
  RISC-V, and neither has been run on silicon here.
