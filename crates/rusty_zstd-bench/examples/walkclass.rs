fn main() {
    #[cfg(feature = "profile")]
    {
        rusty_zstd::set_walk_rep_max_arm(f32::MAX);
        // `set_walk_offrep_max_arm` was called here but has never existed in the
        // crate -- so this instrument has never compiled under `--features
        // profile`, the only configuration it runs in. Dropped, not stubbed:
        // there is no off-rep ceiling arm to pin.
        rusty_zstd::set_walk_cont_arm(true);
        for id in ["jsonlog-16m","smallmsg-8m","dickens","reymont","mr"] {
            let f = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
            let s = &f[..f.len().min(6 << 20)];
            let _ = rusty_zstd::take_walk_classes();
            let _ = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: 12, checksum: false }).unwrap();
            let (first, upg) = rusty_zstd::take_walk_classes();
            println!("{id}: cont-FIRST {first}, cont-UPGRADE {upg}, first-share {:.1}%",
                if first+upg==0 {0.0} else {100.0*first as f64/(first+upg) as f64});
        }
    }
}
