//! XXH64 (seed 0), used for the zstd content checksum (low 32 bits).
//!
//! Spec: https://github.com/Cyan4973/xxHash/blob/v0.8.2/doc/xxhash_spec.md
//! Pure Rust, no_std, no alloc.
//!
//! Primes are from that spec. Do not "correct" P2 against other writeups --
//! `0xC2B2AE3D27D4EB4F` is required for XXH64("", 0) == `0xEF46DB3751D8E999`.

const P1: u64 = 0x9E37_79B1_85EB_CA87;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;
const P4: u64 = 0x85EB_CA77_C2B2_AE63;
const P5: u64 = 0x27D4_EB2F_1656_67C5;

#[inline(always)]
fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2))
        .rotate_left(31)
        .wrapping_mul(P1)
}

#[inline(always)]
fn merge(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val)).wrapping_mul(P1).wrapping_add(P4)
}

#[inline(always)]
fn read_u64_at(s: &[u8], i: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&s[i..i + 8]);
    u64::from_le_bytes(buf)
}

#[inline(always)]
fn read_u32_at(s: &[u8], i: usize) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&s[i..i + 4]);
    u32::from_le_bytes(buf)
}

#[inline(always)]
fn stripe(v1: &mut u64, v2: &mut u64, v3: &mut u64, v4: &mut u64, src: &[u8], off: usize) {
    *v1 = round(*v1, read_u64_at(src, off));
    *v2 = round(*v2, read_u64_at(src, off + 8));
    *v3 = round(*v3, read_u64_at(src, off + 16));
    *v4 = round(*v4, read_u64_at(src, off + 24));
}

/// XXH64 with seed 0 -- the only seed zstd uses for the content checksum.
pub fn xxh64(input: &[u8]) -> u64 {
    xxh64_seed(input, 0)
}

pub fn xxh64_seed(input: &[u8], seed: u64) -> u64 {
    let len = input.len();
    let mut off = 0usize;
    let mut acc: u64;

    if len >= 32 {
        let mut v1 = seed.wrapping_add(P1).wrapping_add(P2);
        let mut v2 = seed.wrapping_add(P2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(P1);
        // Slice a proven 128-byte window so LLVM drops per-load bounds checks
        // (the unrolled index loop emitted 16 cmp+ja per stripe).
        let n128 = (len / 128) * 128;
        for chunk in input[..n128].chunks_exact(128) {
            stripe(&mut v1, &mut v2, &mut v3, &mut v4, chunk, 0);
            stripe(&mut v1, &mut v2, &mut v3, &mut v4, chunk, 32);
            stripe(&mut v1, &mut v2, &mut v3, &mut v4, chunk, 64);
            stripe(&mut v1, &mut v2, &mut v3, &mut v4, chunk, 96);
        }
        let n32 = (len / 32) * 32;
        for chunk in input[n128..n32].chunks_exact(32) {
            stripe(&mut v1, &mut v2, &mut v3, &mut v4, chunk, 0);
        }
        off = n32;
        acc = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        acc = merge(acc, v1);
        acc = merge(acc, v2);
        acc = merge(acc, v3);
        acc = merge(acc, v4);
    } else {
        acc = seed.wrapping_add(P5);
    }

    acc = acc.wrapping_add(len as u64);

    while off + 8 <= len {
        let k1 = round(0, read_u64_at(input, off));
        acc = (acc ^ k1).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        off += 8;
    }
    if off + 4 <= len {
        let k1 = u64::from(read_u32_at(input, off));
        acc = (acc ^ k1.wrapping_mul(P1))
            .rotate_left(23)
            .wrapping_mul(P2)
            .wrapping_add(P3);
        off += 4;
    }
    while off < len {
        acc = (acc ^ u64::from(input[off]).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
        off += 1;
    }

    acc ^= acc >> 33;
    acc = acc.wrapping_mul(P2);
    acc ^= acc >> 29;
    acc = acc.wrapping_mul(P3);
    acc ^= acc >> 32;
    acc
}

/// Incremental XXH64 (seed 0), matching [`xxh64`] on the concatenated input.
pub struct Xxh64 {
    total: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    v4: u64,
    buf: [u8; 32],
    buf_len: usize,
    large: bool,
}

impl Xxh64 {
    /// Seed 0 -- the only seed zstd uses for the content checksum.
    pub fn new() -> Self {
        Self {
            total: 0,
            v1: P1.wrapping_add(P2),
            v2: P2,
            v3: 0,
            v4: 0u64.wrapping_sub(P1),
            buf: [0; 32],
            buf_len: 0,
            large: false,
        }
    }

    /// Absorb more bytes.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (32 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 32 {
                let chunk = self.buf;
                self.consume_stripe(&chunk);
                self.large = true;
                self.buf_len = 0;
            }
        }
        while data.len() >= 128 {
            let chunk = &data[..128];
            self.consume_stripe(&chunk[..32]);
            self.consume_stripe(&chunk[32..64]);
            self.consume_stripe(&chunk[64..96]);
            self.consume_stripe(&chunk[96..128]);
            self.large = true;
            data = &data[128..];
        }
        while data.len() >= 32 {
            self.consume_stripe(&data[..32]);
            self.large = true;
            data = &data[32..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn consume_stripe(&mut self, chunk: &[u8]) {
        stripe(
            &mut self.v1,
            &mut self.v2,
            &mut self.v3,
            &mut self.v4,
            chunk,
            0,
        );
    }

    /// Current digest of all bytes absorbed so far.
    pub fn digest(&self) -> u64 {
        let mut rest = &self.buf[..self.buf_len];
        let mut acc = if self.large {
            let mut a = self
                .v1
                .rotate_left(1)
                .wrapping_add(self.v2.rotate_left(7))
                .wrapping_add(self.v3.rotate_left(12))
                .wrapping_add(self.v4.rotate_left(18));
            a = merge(a, self.v1);
            a = merge(a, self.v2);
            a = merge(a, self.v3);
            merge(a, self.v4)
        } else {
            P5
        };
        acc = acc.wrapping_add(self.total);
        while rest.len() >= 8 {
            let k1 = round(0, read_u64_at(rest, 0));
            acc = (acc ^ k1).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
            rest = &rest[8..];
        }
        if rest.len() >= 4 {
            let k1 = u64::from(read_u32_at(rest, 0));
            acc = (acc ^ k1.wrapping_mul(P1))
                .rotate_left(23)
                .wrapping_mul(P2)
                .wrapping_add(P3);
            rest = &rest[4..];
        }
        while let Some((&b, tail)) = rest.split_first() {
            acc = (acc ^ u64::from(b).wrapping_mul(P5))
                .rotate_left(11)
                .wrapping_mul(P1);
            rest = tail;
        }
        acc ^= acc >> 33;
        acc = acc.wrapping_mul(P2);
        acc ^= acc >> 29;
        acc = acc.wrapping_mul(P3);
        acc ^= acc >> 32;
        acc
    }
}

impl Default for Xxh64 {
    fn default() -> Self {
        Self::new()
    }
}

/// Low 32 bits, little-endian -- the 4-byte zstd content checksum field.
pub fn content_checksum(data: &[u8]) -> u32 {
    xxh64(data) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_seed0() {
        // Published XXH64("", 0).
        assert_eq!(xxh64(b""), 0xEF46_DB37_51D8_E999);
        assert_eq!(content_checksum(b""), 0x51D8_E999);
    }

    #[test]
    fn c_zstd_157_checksums() {
        // facebook/zstd v1.5.7 CLI frames (see decode tests).
        assert_eq!(content_checksum(b"a"), 0xA98C_6E5B);
        assert_eq!(content_checksum(b"hello"), 0x889F_6DA3);
    }

    #[test]
    fn thirty_two_zeros_stripe_path() {
        assert_eq!(xxh64(&[0u8; 32]), 0xF6E9_BE5D_7063_2CF5);
    }

    #[test]
    fn hasher_matches_oneshot() {
        let samples: &[&[u8]] = &[
            b"", b"a", b"hello", &[0u8; 31], &[0u8; 32], &[0u8; 33], &[0u8; 64],
        ];
        for &s in samples {
            let mut h = Xxh64::new();
            h.update(s);
            assert_eq!(h.digest(), xxh64(s), "len {}", s.len());
            let mut h2 = Xxh64::new();
            for chunk in s.chunks(3) {
                h2.update(chunk);
            }
            assert_eq!(h2.digest(), xxh64(s), "chunked len {}", s.len());
        }
        let mut long = vec![0u8; 100_003];
        for (i, b) in long.iter_mut().enumerate() {
            *b = (i.wrapping_mul(251) % 251) as u8;
        }
        let mut h = Xxh64::new();
        h.update(&long);
        assert_eq!(h.digest(), xxh64(&long), "long oneshot");
        let mut h2 = Xxh64::new();
        for chunk in long.chunks(17) {
            h2.update(chunk);
        }
        assert_eq!(h2.digest(), xxh64(&long), "long chunked");
    }
}

#[cfg(test)]
mod locality_probe {
    use super::*;
    use std::time::Instant;

    /// Is our xxh64 COMPUTE-bound or MEMORY-bound? If hashing a cache-resident
    /// buffer is much faster per byte than hashing a 32 MiB one, the checksum
    /// is limited by reading cold memory -- and fusing it into the decode loop
    /// (hashing each block while it is still hot) would be a real win.
    #[ignore]
    #[test]
    fn xxh64_throughput_by_working_set() {
        for (label, sz) in [
            ("32 KiB (L1/L2)", 32usize << 10),
            ("256 KiB (L2)", 256 << 10),
            ("4 MiB (L3)", 4 << 20),
            ("32 MiB (DRAM)", 32 << 20),
        ] {
            let buf = alloc::vec![0u8; sz];
            // Equalise total bytes hashed across sizes.
            let total: usize = 512 << 20;
            let reps = total / sz;
            let t = Instant::now();
            let mut acc = 0u64;
            for _ in 0..reps {
                acc ^= u64::from(content_checksum(&buf));
            }
            let s = t.elapsed().as_secs_f64();
            std::hint::black_box(acc);
            println!("  {label:16} {:7.1} GB/s", (total as f64) / s / 1e9);
        }
    }
}
