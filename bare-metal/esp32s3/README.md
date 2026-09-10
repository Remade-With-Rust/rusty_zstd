# rusty_zstd on an ESP32-S3 — the on-board proof, and the heap budget

"Builds for a bare-metal target" and "runs on the part" are different claims.
CI makes the first one every push (`cargo check --target thumbv7em-none-eabihf`
and `--target riscv32imac-unknown-none-elf`). This directory is the second one,
and it is hand-run: it needs Espressif's Rust fork, which CI does not have.

**Result, 2026-09-09, ESP32-S3 revision v0.2, 8 MB flash:**

```
=== rusty_zstd on ESP32-S3 (xtensa, no_std + alloc) ===
allocator         esp-alloc 0.11.0
census64::CENSUS_LIVE = false  (false is expected here: no 64-bit atomics)
source            8260 bytes
L1  8260 ->  468 bytes  (17.65x)  round trip OK
L3  8260 ->  426 bytes  (19.39x)  round trip OK
L5  8260 ->  263 bytes  (31.41x)  round trip OK
RESULT: PASS -- compressed and decompressed on the board
```

Three levels because they are three different match finders: L1 is Fast, L3 is
DFast, L5 is Greedy. Each was decompressed and compared byte for byte against
the source, on the part.

## Why this part is the right test

Xtensa LX7 is 32-bit, so `core::sync::atomic::AtomicU64` does not exist on it —
the same reason Cortex-M4F and RV32 could not build the crate at all until
`census64` landed. Every census counter here is the zero-sized stub, which is
why `CENSUS_LIVE` prints `false`. **A zero from a counter on this part means
"not measurable on this target", never "measured zero".**

## The heap budget, measured

The number a firmware author actually needs. Peak is what the allocator reports
having in use; the floor is the smallest heap that still round-trips, found by
shrinking until it fails.

| workload | peak heap | smallest heap that works |
|---|---|---|
| L1 only | 112,468 B | **112 KiB** (110 KiB fails) |
| L1 + L3 + L5 | 175,832 B | **176 KiB** (172 KiB fails) |

**The jump is DFast, not depth.** L1 peaks at 112,468 and L3 at 175,832; L5 adds
nothing over L3. So the second hash table DFast introduces costs ~62 KiB here,
and a firmware that can live with L1 saves that. Compression is barely worse:
17.65x against 19.39x on this corpus.

Everything is transient: `current` after a round trip is 41,292 bytes, so the
tables are freed and the budget is a peak, not a resident cost.

## Two allocator arms, one source

```sh
cargo run --release                                            # esp-alloc
cargo run --release --no-default-features -F arm-rusty-alloc   # rusty_alloc
```

`rusty_alloc` additionally needs `--cfg ra_single_threaded --cfg
ra_small_profile`, both already in `.cargo/config.toml`. Without the second,
`SEGMENT_SIZE` stays at 32 MiB, a chip-scale region yields zero segments, and
every allocation fails with nothing to tell you why.

**The rusty_alloc arm does not currently work on this part**, and the reason is
not the codec:

| | esp-alloc 0.11 | rusty_alloc 2.0.5 |
|---|---|---|
| round trip at 192 KiB | **PASS** | OOM on a 1,536 B allocation |
| round trip at 256 KiB | n/a | OOM on a 656 B allocation |
| 320 KiB region | n/a | does not link, "Main stack is smaller than 8192 bytes" |

Reduced to a two-line reproduction with no zstd in it: in a **256 KiB region,
rusty_alloc serves exactly one 64 KiB allocation**, and the second fails with
192 KiB of the region still free. Blocks of 32 KiB pack fine (four of them, half
the region). The cliff is at the segment size.

RAM on this part is a fixed map, so a larger region comes straight out of
`.stack` until the linker refuses, which means there is no region size on an S3
where this consumer can use it. Written up for the allocator's maintainers in
`rusty_alloc/docs/plans/esp32-large-alloc-ceiling.md`.

Static cost of the arm, both at the same 192 KiB budget, from the linked ELF:
`.bss` +2,060, `.data` −76, `.stack` −1,996, flash +7,536. `.data + .bss +
.stack` sums to a constant within 12 bytes, so the stack column is the one that
moves — quote the sum, never a `.bss` delta alone.

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

To re-measure the floor, `heapsweep.py` in the session scratchpad patches
`HEAP_BYTES`, rebuilds, flashes and classifies PASS / OOM per point. Note that
`espflash --monitor` never exits: the timeout is the stop signal, and the log is
complete by then. Redirect it to a FILE — a `| tail` loses everything when the
timeout kills the pipe.

## What it does not claim

- **No timing.** There is no cycle count here, so nothing in this directory is a
  performance claim. It answers "does it work" and "how much RAM", not "how
  fast". Buffer placement alone moves compute kernels on this part by up to 20%,
  so a throughput comparison across allocator arms would measure placement.
- **One part.** The S3 is Xtensa. The two targets CI compiles are ARM and
  RISC-V, and neither has been run on silicon here.
