//! Issue #32: a one-shot frame with a dictionary or prefix must not copy the
//! whole input. Long inputs are coded from a `window + 128 KiB` copy and then
//! in place (`encode_handover`), so peak heap stays near the no-dictionary
//! path's instead of one extra input-sized buffer.
//!
//! One `#[test]` only: the counting allocator is process-wide, and a second
//! test running in parallel would land in the peak.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged to the system allocator.
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        // SAFETY: `p` came from `alloc` above with this layout.
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Peak bytes above the live total at entry, and the result.
fn peak_of<T>(f: impl FnOnce() -> T) -> (usize, T) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let r = f();
    (PEAK.load(Ordering::Relaxed) - base, r)
}

/// Text-like input with long-range repeats and a sprinkle of noise.
fn input(len: usize) -> Vec<u8> {
    let words: &[&[u8]] = &[
        b"stream ",
        b"frame ",
        b"window ",
        b"literal ",
        b"offset ",
        b"match ",
        b"entropy ",
        b"huffman ",
        b"sequence ",
        b"block ",
        b"dictionary\n",
    ];
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let mut v = Vec::with_capacity(len + 16);
    while v.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if x % 23 == 0 {
            v.extend_from_slice(&x.to_le_bytes());
        } else {
            v.extend_from_slice(words[(x % words.len() as u64) as usize]);
        }
    }
    v.truncate(len);
    v
}

#[test]
fn long_dict_and_prefix_frames_are_coded_in_place() {
    let src = input(8 << 20);
    let dict_bytes = input(4096);
    let dict = rusty_zstd::Dictionary::raw(dict_bytes.clone());

    // Baseline: the same input without a dictionary.
    let (plain, _) = peak_of(|| rusty_zstd::compress(&src, 1).expect("plain"));

    for level in [1, 3] {
        let (with_dict, zst) =
            peak_of(|| rusty_zstd::compress_using_dict(&src, &dict, level).expect("dict"));
        let back = rusty_zstd::decompress_using_dict(&zst, &dict).expect("dict decode");
        assert!(back == src, "dict L{level} round trip");
        // Before the fix this was `plain + src.len()` (one more input copy).
        assert!(
            with_dict < plain + src.len() / 4,
            "dict L{level}: peak {with_dict} vs no-dict {plain} (input {})",
            src.len()
        );

        let (with_prefix, zst) = peak_of(|| {
            rusty_zstd::compress_using_prefix(&src, &dict_bytes, level).expect("prefix")
        });
        let back = rusty_zstd::decompress_using_prefix(&zst, &dict_bytes).expect("prefix decode");
        assert!(back == src, "prefix L{level} round trip");
        assert!(
            with_prefix < plain + src.len() / 4,
            "prefix L{level}: peak {with_prefix} vs no-dict {plain}"
        );
    }

    // A second call with the same dictionary is the digested path's turn; it
    // must decline this size and hand over too, not copy into its work set.
    let (again, zst) = peak_of(|| rusty_zstd::compress_using_dict(&src, &dict, 1).expect("again"));
    assert!(rusty_zstd::decompress_using_dict(&zst, &dict).expect("again decode") == src);
    assert!(
        again < plain + src.len() / 4,
        "repeat call: peak {again} vs {plain}"
    );

    // Checksummed frames hash both halves.
    let opts = rusty_zstd::CompressOptions {
        level: 3,
        checksum: true,
    };
    let zst = rusty_zstd::compress_using_dict_with(&src, &dict, opts, true).expect("checksum");
    assert!(rusty_zstd::decompress_using_dict(&zst, &dict).expect("checksum decode") == src);
}
