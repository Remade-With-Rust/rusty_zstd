//! Confirm the shipped default now SKIPS, and that forcing the old write back
//! reproduces the same bytes (the fallback proof the ledger requires).
fn main() {
    let mut n = 0; let mut diff = 0;
    for id in ["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"] {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(1 << 20)];
        let (pre, tail) = s.split_at(s.len()/2);
        for lvl in [13i32, 19, 22] {
            let d = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();   // default
            rusty_zstd::set_prime_bt_arm(true);                                    // old behaviour
            let k = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            rusty_zstd::set_prime_bt_arm(false);
            let sk = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            assert_eq!(d, sk, "{id} L{lvl}: default is NOT the skip arm");
            assert!(rusty_zstd::decompress_using_prefix(&d, pre).unwrap() == tail, "{id} L{lvl} round-trip");
            n += 1;
            if d != k { diff += 1; }
        }
    }
    println!("{n} cells: default == skip arm on all; default vs kept-write differs on {diff} (expect 0)");
    assert_eq!(diff, 0, "fallback is NOT byte-identical");
}
